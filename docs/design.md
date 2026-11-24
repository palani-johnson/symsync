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
- **Package:** a named group of links, defined in a repo config. The package is the unit of ownership. Package names are unique in a state directory.
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

Any link that resolves through `<state dir>/links/` is managed by symsync. A link that points straight into a repo is not, unless it is a [direct link](#direct-links).

## Direct links

Some apps follow only one level of symlinks when they watch a file for changes, so an edit in the repo does not reach the app through the two-link chain. For these files, a link group can set `direct = true`. Then the target link points straight to the source, and the target has no state link:

```
~/.claude/settings.json  →  ~/dotfiles/claude/settings.json
       target link                     source
```

A direct link gives up two properties of the chain:

- It is not self-identifying. Only `packages.toml` shows that symsync manages it.
- A repo move or a source change rewrites the target link itself, not a state link.

All other rules stay the same. The packages file records each direct target with `direct = true`, and the [link type](#link-type) sets whether the target link is absolute or relative. Sync removes or replaces a direct target link only when it points to the recorded source. A link that already points to the source, but that symsync did not create, is a conflict, as for any other foreign link.

When a target changes between direct and not direct, sync replaces the target link. When a target becomes direct, sync removes its state link after it replaces the target link.

## Link type

The link type sets how symsync stores every path for a package:

- `absolute` (default): target links, state links and the paths in `packages.toml` are absolute.
- `relative`: target links point to state links by relative path, state links point to sources by a path relative to `links/`, and `packages.toml` records paths relative to the state directory.

Relative mode makes a link chain that is fully self-contained. You can commit it, and it works after a clone to any location. The examples folder uses this mode. A relative chain survives a move only when everything moves together, so in relative mode the state directory and all sources and targets must be inside the config directory. If one is outside, `plan` and `sync` stop with an error.

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
    <sha256 of target path>   → source
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

[packages.claude]
config = "/Users/uzz/dotfiles/symsync.toml"
link_type = "absolute"

[packages.claude.targets."/Users/uzz/.claude/settings.json"]
source = "/Users/uzz/dotfiles/claude/settings.json"
direct = true
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
sources = [".zshrc", ".zshenv"]
target = "~"

[packages.zed]
[[packages.zed.links]]
sources = ["*.json"]
target = "~/.config/zed"
```

The config has these top-level settings, which are all optional:

- `state_dir`: the state directory, relative to the config file (see [State directory](#state-directory)).
- `link_type`: `absolute` or `relative` (see [Link type](#link-type)).
- `git_tracked`: link only files that git tracks (see [Git-tracked files](#git-tracked-files)).
- `direct`: link targets straight to their sources (see [Direct links](#direct-links)).

A package has `links`, its list of link groups, and optional `git_tracked` and `direct` values that override the top-level values.

Each link group has these fields:

- `root`: the directory that the source globs are relative to. The default is the directory with the same name as the package, next to the config file. To use the config directory, set `root = "."`.
- `sources`: globs that select source files. The default is `["**/*"]`, every file in `root`.
- `target`: the target directory. Each match goes to `target` joined with the path of the match relative to `root`. For example, `zed/themes/dark.json` with `root = "zed"` goes to `~/.config/zed/themes/dark.json`.
- `rename`: an ordered list of rules that give sources a different path at the target (see [Rename](#rename)).
- `git_tracked`: overrides the package and top-level values (see [Git-tracked files](#git-tracked-files)).
- `direct`: overrides the package and top-level values. The first value that is set applies, and the default is `false`.

Globs use the [wax](https://github.com/olson-sean-k/wax) syntax. A glob must be relative to `root`, so a glob that starts with `/` or has a `.` or `..` component is an error. Globs select only files. Symsync does not link directories. Wildcards also match names that start with `.`. With the default `git_tracked = true`, the files in `.git/` are still never sources.

### Rename

By default, a target has the same path as its source, relative to `root`. A rename rule has two fields:

- `from`: a glob that matches the source path relative to `root`.
- `to`: the target path relative to `target`. `{n}` is the text of capture `n` of `from`, and `{0}` is the whole source path. Each wildcard, class, alternation and repetition in `from` is one capture, numbered from 1 in the order they occur. `{{` and `}}` are literal braces.

For each source, symsync uses only the first rule whose `from` matches. A source that no rule matches keeps its path. Rename rules do not change which files are sources.

```toml
[[packages.zsh.links]]
target = "~"
rename = [
  { from = "zshrc.local", to = ".zshrc" },
  { from = "**/dot-*", to = "{1}.{2}" },
]
```

Here `zshrc.local` links to `~/.zshrc`, `dot-zshenv` links to `~/.zshenv`, and `dir/dot-profile` links to `~/dir/.profile`.

A renamed path must be a relative path inside `target`, so a result that is empty or has a `.` or `..` component is an error. When two sources rename to the same target, sync stops with a conflict. A rule that renames no source gives a warning, like a glob that matches no files.

### Git-tracked files

When `git_tracked` is `true`, which is the default, a link group links only the matches that are in the git index of the repo that holds its `root`, as `git ls-files` shows them. Untracked and ignored files, and the files in `.git/`, are not sources. A staged file counts as tracked before its first commit.

The value comes from the link group, then the package, then the top level. The first one that is set applies, and the default is `true`. When the value is `true`, `git` must be on the `PATH` and `root` must be inside a git repo, or `plan` and `sync` stop with an error. To link files from a directory that is not in a git repo, set `git_tracked = false`. When the filter removes every match of a glob, symsync shows the same warning as for a glob that matches no files.

Some files are never sources, even when a glob matches them:

- The config file.
- Files in the state directory.
- Links into the state directory. These are targets that symsync manages. Without this rule, a target directory inside a `root` feeds its own links back in as new sources.

When a glob matches no files, `plan` and `sync` show a warning. A typo in a glob or a renamed `root` otherwise makes sync unlink the whole package without notice.

## Commands

- `symsync plan`: shows the changes that `sync` would make. It does not change anything.
- `symsync sync`: applies the plan for all packages in the current config.
- `symsync status`: shows every package in the state directory, its config, its targets, the health of each target (see [Health checks](#health-checks)), and stray state links.
- `symsync remove <package>`: removes a package and all its links, with only the state as input. The config does not need to exist.
- `symsync prune`: removes stray state links (see [Stray state links](#stray-state-links)).
- `symsync adopt <target>`: recovers a target that an app replaced with a regular file (see [Drift](#drift)).
- `symsync config`: prints the parsed config.
- `symsync gen-config-schema`: prints the JSON schema of the config.

Commands that read a config find `symsync.toml` in the current directory, or use the path given with `--config`. All commands accept `--state-dir`.

`plan` and `sync` accept `--package <name>` (or `-p`), one or more times. With it, symsync plans only the named packages and leaves all other packages alone, even packages that the config no longer defines.

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
5. Before any change, write each changed package to `packages.toml` with the union of its old and new targets. A target in both keeps its old record. Then apply the changes, write the final records, and release the lock.

Because of step 5, every link that can be on disk is recorded, even when a change fails or the process stops. The next sync then removes any extra links with the usual safety rule. A target link of a recorded target also counts as managed when it points straight to its recorded or its new source, because a sync that stops partway can leave either one while it switches a target to or from a direct link.

When the last entry in a directory is unlinked, sync shows a note. Symsync does not remove directories.

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

`status` shows one of these for each target:

- **ok:** the full chain from target to source is correct.
- **missing link:** there is nothing at the target. Sync creates the link again.
- **drifted:** the target is a regular file (see [Drift](#drift)). Sync reports a conflict.
- **exists:** the target is a directory or another file that is not a link. Sync reports a conflict.
- **foreign link:** the target is a link that does not point to its state link, or to its source for a direct link. Sync reports a conflict.
- **missing state link:** the state link does not exist. Sync creates it again. Not used for direct links.
- **wrong state link:** the state link does not point to the recorded source. Sync rewrites it. Not used for direct links.
- **missing source:** the source file does not exist. Its glob no longer matches it, so sync unlinks the target.

## Stray state links

A stray state link is an entry in `links/` that no package records, or the state link of a direct target, for example after manual changes to the state directory. `status` lists stray state links, and `symsync prune` removes them. Prune removes only symlinks, and leaves other files in place with a note.

## Paths

Symsync canonicalizes the config path and the state directory (it resolves symlinks in the part of the path that exists). Thus a config or state directory has one identity, however you reach it. Targets and sources are only normalized lexically, so that a target keeps the path that you wrote in the config.

## Known limitations

Symsync is a tool to manage dotfiles for one user. These edge cases are known, and they are not likely in normal use, so symsync does not handle them:

- **Relative links under a symlinked parent:** in relative mode, symsync computes link values from the path text. If a target's parent directory is a symlink, the kernel resolves the link from the real directory, and the link can dangle while `status` reports `ok`.
- **`plan` and `sync` can disagree in rare layouts:** if one link group creates a file at a path that another link group uses as a directory, or if a parent directory of a target is a dangling symlink, `plan` shows no conflict but `sync` fails partway. Because sync records old and new targets before any change, the next sync recovers.
- **Ownership checks ignore other state directories:** when symsync checks whether another config still defines a package, it does not check that the other config uses the same state directory.
- **`--package` still expands all packages:** a bad glob or a target collision in another package also stops a filtered run.
- **`~` in `adopt`:** the shell expands `~`. A quoted `'~/x'` is not expanded.
- **Unicode normalization:** state link names hash the bytes of the target path. On macOS, the same name in NFC and NFD form gives two different state links.
- **`$SYMSYNC_STATE_DIR` overrides the config:** when it is set in your shell, it also replaces `state_dir` in a config, for example in the examples folder.

## Testing

Integration tests run the binary in a temp directory with `$SYMSYNC_STATE_DIR` and `$HOME` set to temp paths. Each test has its own state and cannot affect the real machine.

## Future work

- **Machine-specific package selection:** a machine-local file that selects which packages are active on the machine. The `--package` filter is a first step.
- **Directory links:** link a whole directory to capture files that an app creates. Files stay the default. The planner must then stop file links from going inside a linked directory.
