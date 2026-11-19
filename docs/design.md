# Symsync design

This document describes the design of symsync.

## Goals

- Manage dotfiles as live symlinks. A file must be editable on both ends: an edit in the repo shows in the app, and an edit made from the app (for example, Zed's settings UI) lands in the repo.
- Make every managed link self-identifying. A person who resolves a link must be able to see that symsync manages it, without knowledge of the repo layout.
- Keep an explicit record of what symsync owns, so that it never guesses and never removes something it did not create.
- Survive repo moves and re-clones without changes to the links in the home directory.
- Support many repos on one machine, each with many packages.
- Show a plan before any change.

## Non-goals

- Templating, or any deploy that copies files. Both break the "editable on both ends" goal.
- Windows support.
- Secrets management.

## Concepts

- **Repo:** a directory with a `symsync.toml` config file. A machine can have many repos.
- **Package:** a named group of links, defined in a repo config. The package is the unit of ownership. Package names are unique on the machine.
- **Source:** a file in a repo.
- **Target:** the path where the user expects the file, for example `~/.config/zed/settings.json`.
- **Target link:** the symlink at the target. It points to a state link.
- **State link:** a symlink in the state directory. It points to the source.
- **State directory:** a machine-local directory that holds all state links and the packages file.

## Link chain

```
~/.config/zed/settings.json  →  ~/.local/state/symsync/links/<sha256>  →  ~/dotfiles/zed/settings.json
       target link                        state link                             source
```

The target link never changes after symsync creates it. All changes, such as a repo move or a new source for a target, rewrite only the state link. Symsync controls the state directory fully, so these rewrites are safe.

Any link that resolves through `<state dir>/links/` is managed by symsync. A link that points straight into a repo is not.

## Link type

The link type sets how symsync stores every path for a package:

- `absolute` (default): target links, state links and the paths in `packages.toml` are absolute.
- `relative`: target links point to state links by relative path, state links point to sources by a path relative to `links/`, and `packages.toml` records paths relative to the state directory.

Relative mode makes a link chain that is fully self-contained when the state directory is inside the repo. You can commit it, and it works after a clone to any location. The examples folder uses this mode.

Symsync finds the link type in this order:

1. `$SYMSYNC_LINK_TYPE`
2. `link_type` in the config
3. `absolute`

## State directory

Symsync finds the state directory in this order:

1. The `--state-dir` flag
2. `$SYMSYNC_STATE_DIR`
3. `state_dir` in the config, relative to the config file
4. `$XDG_STATE_HOME/symsync`
5. `~/.local/state/symsync`

The XDG path is also used on macOS. Tests set `$SYMSYNC_STATE_DIR` to a temp directory.

Package names are unique in one state directory. A package in a different state directory is not visible from the default state directory. To find or remove it without its config, you must give its state directory with `--state-dir` or `$SYMSYNC_STATE_DIR`.

Layout:

```
<state dir>/
  links/
    <sha256 of target path>   → absolute path of source
  packages.toml
  lock
```

### State link names

The name of a state link is the SHA-256 hex digest of the normalized target path, relative to the state directory. To normalize a path, symsync expands `~`, makes the path absolute, and cleans it lexically (it removes `.`, `..` and duplicate separators). Symsync does not resolve symlinks during normalization. Without normalization, `~/.zshrc` and `/Users/uzz/.zshrc` get different names and the one-owner rule fails.

The path is relative to the state directory so that the name stays the same when a repo and its state directory move together, as in relative mode. In absolute mode, the targets and the state directory do not move, so the names also stay the same.

Because the name comes from the target, each target has at most one state link, and the file system enforces one owner per target.

State links point to the source by absolute or relative path, as the [link type](#link-type) sets.

### Packages file

`packages.toml` is the record of ownership for all packages on the machine.

```toml
version = 1

[packages.zsh]
config = "/Users/uzz/dotfiles/symsync.toml"
link_type = "absolute"

[packages.zsh.targets."/Users/uzz/.zshrc"]
source = "/Users/uzz/dotfiles/zsh/.zshrc"

[packages.zed]
config = "/Users/uzz/dotfiles/symsync.toml"
link_type = "absolute"

[packages.zed.targets."/Users/uzz/.config/zed/settings.json"]
source = "/Users/uzz/dotfiles/zed/settings.json"
```

Each package records its link type. For a `relative` package, the config, target and source paths are relative to the state directory.

The packages file holds all the data that is necessary to remove a package without its config. This is what makes recovery possible (see [Package recovery](#package-recovery)).

Symsync writes the packages file atomically: it writes a temp file and then renames it. Every command that changes state first takes an exclusive lock on `<state dir>/lock`, so two syncs that run at the same time cannot corrupt the file.

## Repo config

Each repo has a `symsync.toml` file at its root (json, json5, jsonc, and yaml are also supported). Paths in the config are relative to the config file.

```toml
state_dir = ".symsync.state"
link_type = "relative"

[packages.zsh]
[[packages.zsh.links]]
root = "zsh"
sources = [".zshrc", ".zshenv"]
target = "~"

[packages.zed]
[[packages.zed.links]]
root = "zed"
sources = ["*.json"]
target = "~/.config/zed"
```

The config has these top-level settings, which are both optional:

- `state_dir`: the state directory, relative to the config file (see [State directory](#state-directory)).
- `link_type`: `absolute` or `relative` (see [Link type](#link-type)).

Each link group has these fields:

- `root`: the directory that the source globs are relative to. The default is the config directory.
- `sources`: globs that select source files.
- `target`: the target directory. Each match goes to `target` joined with the path of the match relative to `root`. For example, `zed/themes/dark.json` with `root = "zed"` goes to `~/.config/zed/themes/dark.json`.
- `hidden`: when `true`, wildcards also match names that start with `.`. The default is `false`: wildcards skip hidden files and directories (so `**/*` does not go into `.git/`), and only a literal `.` in a glob matches them, as in `.zshrc`, `.*` or `.config/**/*`.

Globs select only files. Symsync does not link directories. A target always has the same name as its source, relative to `root`. There is no syntax to map one source to a target with a different name.

## Commands

- `symsync plan`: shows the changes that `sync` would make. It does not change anything.
- `symsync sync`: applies the plan for all packages in the current config.
- `symsync status`: shows every package on the machine, its config, its targets, and the health of each target.
- `symsync remove <package>`: removes a package and all its links, with only the state as input. The config does not need to exist.
- `symsync adopt <target>`: recovers a target that an app replaced with a regular file (see [Drift](#drift)).
- `symsync config`: prints the parsed config.
- `symsync gen-config-schema`: prints the JSON schema of the config.

Commands that read a config find `symsync.toml` in the current directory, or use the path given with `--config`. All commands accept `--state-dir`.

## Sync

Sync does these steps for the packages in one config:

1. Take the lock and read `packages.toml`.
2. For each package in the config, find its owner (see [Package ownership](#package-ownership)). Stop on a conflict.
3. Find the packages in the state whose recorded config is this config but which the config no longer defines. These packages were removed, so sync removes all their links.
4. For each package, compare the targets in the config with the targets in the state:
   - **New target:** create the state link, create the parent directories of the target, and create the target link.
   - **Removed target:** remove the target link (only if it still points to the expected state link), then remove the state link. If the target is no longer a symsync link, sync leaves it in place and shows a note.
   - **Changed source:** rewrite the state link only.
   - **Unchanged target:** check the health of both links. Rewrite the state link if the repo moved.
5. Write `packages.toml` and release the lock.

Sync never removes or overwrites a regular file or a link that symsync does not own. These cases are conflicts. `plan` shows them, and `sync` stops before any change.

`plan` and `sync` use the same code to make the plan, so `plan` always shows exactly what `sync` will do.

## Package ownership

A package name can be in only one live config at a time. When a config defines package `P`, symsync compares the config with the state:

- **`P` is not in the state:** the config claims `P`.
- **The state records this config for `P`:** normal case.
- **The state records a different config, and that config no longer exists or no longer defines `P`:** the package moved. The config claims `P`, and sync rewrites the state links of `P` to point into the new location. `plan` shows "package `P` moved from A to B".
- **The state records a different config, and that config still defines `P`:** conflict. Sync stops. You must rename one of the packages, or remove `P` from the other config.

## Package recovery

A package must always be findable and removable. The packages file records the config, the source and the target of every link, so symsync can find a package from the state alone, without a config.

`status` marks a package as **orphaned** when its recorded config does not exist or no longer defines the package. To recover an orphaned package, do one of these:

- Run `sync` from the new config, if the package moved. The move rule above claims the package.
- Run `symsync remove <package>`. This removes all target links and state links of the package, with the state as the only input, and then removes the package from `packages.toml`. It applies the same safety rule as sync: it removes a target link only if the link still points to the expected state link.

## Drift

Many apps save a file atomically: they write a temp file and rename it over the path. This replaces the target link with a regular file. After this, edits in the app no longer reach the repo, and the app shows no error.

Symsync knows what must be at each target, so it can detect this case:

- `plan` and `status` show a target that is now a regular file as **drifted**. This is different from an ordinary conflict, because symsync created the link there before.
- `symsync adopt <target>` copies the content of the drifted file over the source in the repo, then restores the target link. Version control then shows the change in the repo, and you can review it.

Symsync never adopts a file automatically.

## Health checks

`status` and `plan` report these problems for each target:

- **Missing target link:** symsync recreates it on sync.
- **Drifted:** the target is a regular file (see [Drift](#drift)).
- **Foreign link:** the target is a link that does not point to the expected state link. This is a conflict.
- **Broken state link:** the source does not exist. If the source came from a glob, sync removes the target. If not, sync reports an error.

## Testing

Integration tests run the binary in a temp directory with `$SYMSYNC_STATE_DIR` and `$HOME` set to temp paths. Each test has its own state and cannot affect the real machine.

## Future work

- **Machine-specific package selection:** a machine-local file that selects which packages are active on the machine.
- **Directory links:** link a whole directory to capture files that an app creates. Files stay the default. The planner must then stop file links from going inside a linked directory.
