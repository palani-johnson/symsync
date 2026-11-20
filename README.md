# Symsync - A stateful symlink farm manager

Symsync is a simple, modern symlink farm manager and an alternative to [GNU Stow](https://www.gnu.org/software/stow/). It aims to fill the gap between stow and more advanced tools like [chezmoi](https://www.chezmoi.io/). Symsync uses only live symlinks, so you can edit a file in the repo or from the app that uses it, and both stay the same.

Each target links to a state link in a machine-local state directory, and the state link links to the source in your repo. This makes every managed link self-identifying, lets symsync safely remove links it created, and lets a repo move without changes to the links in your home directory. Links are grouped into named packages, and one machine can use packages from many repos.

See the [design document](docs/design.md) for the full design.

## Installing

Symsync runs on macOS and Linux. It is not on crates.io yet, so you install it from source with Cargo. You need Rust 1.89 or later.

To install the latest version from GitHub:

```sh
cargo install --git https://github.com/palani-johnson/symsync
```

To install from a local clone:

```sh
git clone https://github.com/palani-johnson/symsync
cd symsync
cargo install --path .
```

Both commands put the `symsync` binary in `~/.cargo/bin`. Make sure that this directory is on your `PATH`.

To install with Nix, add `github:palani-johnson/symsync` as a flake input and use `packages.<system>.default`. The flake also has `packages.<system>.schema`, the JSON schema of the config file. To run symsync without an install:

```sh
nix run github:palani-johnson/symsync
```

## How it works

Every managed file is a chain of two links:

```
~/.config/zed/settings.json  →  ~/.local/state/symsync/links/<sha256>  →  ~/dotfiles/zed/settings.json
       target link                        state link                             source
```

The target link never changes after symsync creates it. When a repo moves or a source changes, symsync rewrites only the state link. Any link that resolves through the `links/` directory of a state directory is managed by symsync.

The state directory also holds `packages.toml`, the record of which package owns which targets. With this record, symsync removes only links that it created, and it can remove a package even after its repo is gone. Symsync finds the state directory in this order: the `--state-dir` flag, `$SYMSYNC_STATE_DIR`, `state_dir` in the config, `$XDG_STATE_HOME/symsync`, and `~/.local/state/symsync`.

A package is a named group of links in a repo config, and package names are unique in a state directory. One machine can use packages from many repos, and if two repos define the same package name, sync stops with a conflict.

By default, all links and recorded paths are absolute. With `link_type = "relative"` in the config (or `$SYMSYNC_LINK_TYPE`), they are all relative. Together with a `state_dir` inside the repo, this makes a self-contained chain that you can commit and that works after a clone to any location. In relative mode, the state directory and all targets must be inside the repo. The [basic example](examples/basic) uses this mode.

Symsync is made for dotfiles, and it does not try to handle every edge case. The design document lists the [known limitations](docs/design.md#known-limitations).

## Configuration

Put a `symsync.toml` file at the root of your dotfiles repo (json, json5, jsonc, and yaml are also supported). Each package has one or more link groups. A link group selects files in `root` with globs, and links each file to the same relative path in `target`.

```toml
[[packages.zsh.links]]
root = "zsh"
sources = [".*"]
target = "~"

[[packages.zed.links]]
root = "zed"
sources = ["**/*.json"]
target = "~/.config/zed"
```

Globs use the [wax](https://github.com/olson-sean-k/wax) syntax and must be relative to `root`. Wildcards do not match names that start with `.`, so `**/*` does not go into `.git/`. A literal `.` in a glob matches them, as in `.*` above. To let wildcards match hidden files too, set `hidden = true` on the link group.

To give a source a different path at the target, add `rename` rules to the link group. For each source, only the first rule whose `from` glob matches applies. In `to`, `{n}` is the text of capture `n` of `from`. Each wildcard, class, alternation and repetition is one capture, numbered from 1.

```toml
[[packages.zsh.links]]
root = "zsh"
sources = ["*"]
target = "~"
rename = [{ from = "dot-*", to = ".{1}" }]
```

This links `zsh/dot-zshrc` to `~/.zshrc`. See the [design document](docs/design.md#rename) for the full rules.

The config file, the state directory, and links that symsync manages are never sources. When a glob matches no files, `plan` and `sync` show a warning, because a typo or a renamed `root` otherwise unlinks the whole package:

```
$ symsync plan
warning: package zed: glob **/*.json in ~/dotfiles/zedd matched no files
```

## Usage

The examples below use the config above, in a repo at `~/dotfiles`.

### Plan and sync

`plan` shows the changes without making them. `sync` prints the same lines as it applies them.

```
$ symsync sync
mkdir   ~/.config/zed
link    ~/.config/zed/settings.json -> ~/dotfiles/zed/settings.json
mkdir   ~/.config/zed/themes
link    ~/.config/zed/themes/dark.json -> ~/dotfiles/zed/themes/dark.json
link    ~/.zshenv -> ~/dotfiles/zsh/.zshenv
link    ~/.zshrc -> ~/dotfiles/zsh/.zshrc
```

A later `sync` with nothing to change prints `already in sync`.

```
$ symsync sync
already in sync
```

### Status

`status` shows every package in the state directory and the health of each target.

```
$ symsync status
state: ~/.local/state/symsync
zed (~/dotfiles/symsync.toml)
  ok                 ~/.config/zed/settings.json
  ok                 ~/.config/zed/themes/dark.json
zsh (~/dotfiles/symsync.toml)
  ok                 ~/.zshenv
  drifted            ~/.zshrc
```

### Conflicts

Symsync never removes or overwrites a file or link that it does not own. A conflict stops `plan` and `sync` before any change.

```
$ symsync plan
conflict: ~/.zshrc exists and is not managed by symsync
Error: sync is blocked by 1 conflict(s)
```

### Drift and adopt

Many apps save a file by writing a temp file and renaming it over the path. This replaces the link with a regular file, and the change no longer reaches the repo. Symsync reports this as drift, and `adopt` moves the file back into the repo and restores the link.

```
$ symsync sync
conflict: ~/.zshrc was replaced by a regular file, use `symsync adopt` to recover it
Error: sync stopped by 1 conflict(s), nothing was changed

$ symsync adopt ~/.zshrc
adopt   ~/.zshrc -> ~/dotfiles/zsh/.zshrc
link    ~/.zshrc -> ~/dotfiles/zsh/.zshrc
```

### Removed sources and packages

When a source or a package is no longer in the config, `sync` removes its links. A target that is no longer a symsync link stays in place.

```
$ symsync sync
unlink  ~/.config/zed/themes/dark.json
note: ~/.config/zed/themes has no entries left, symsync does not remove directories

$ symsync sync
note: package zed was removed from the config
note: ~/.config/zed/settings.json is no longer managed, it was left in place because it is not a symsync link
```

### Moved repos

After you move a repo, run `sync` from the new location. Only the state links change, so the links in your home directory stay the same.

```
$ mv ~/dotfiles ~/dots
$ cd ~/dots
$ symsync sync
relink  ~/.zshenv -> ~/dots/zsh/.zshenv
relink  ~/.zshrc -> ~/dots/zsh/.zshrc
note: package zsh moved from ~/dotfiles/symsync.toml to ~/dots/symsync.toml
```

### Orphaned packages

If a repo is gone, `status` marks its packages as orphaned. `remove` removes a package and its links with only the state as input.

```
$ symsync status
state: ~/.local/state/symsync
zsh (orphaned, ~/dots/symsync.toml)
  missing source     ~/.zshenv
  missing source     ~/.zshrc

$ symsync remove zsh
unlink  ~/.zshenv
unlink  ~/.zshrc
removed package zsh
```

### Selected packages

`plan` and `sync` accept `--package` (or `-p`), one or more times. Symsync then plans only the named packages and leaves all others alone.

```
$ symsync sync --package zsh
link    ~/.zshenv -> ~/dotfiles/zsh/.zshenv
link    ~/.zshrc -> ~/dotfiles/zsh/.zshrc
```

### Stray state links

A state link that no package records is stray, for example after manual changes to the state directory. `status` lists stray state links, and `prune` removes them.

```
$ symsync status
state: ~/.local/state/symsync
zsh (~/dotfiles/symsync.toml)
  ok                 ~/.zshenv
  ok                 ~/.zshrc
stray state links, run `symsync prune` to remove them
  ~/.local/state/symsync/links/3f2a9c…

$ symsync prune
remove  ~/.local/state/symsync/links/3f2a9c…
```

### Config and schema

`config` prints the parsed config as JSON, and `gen-config-schema` prints the JSON schema of the config format.
