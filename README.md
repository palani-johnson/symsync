# Symsync - A stateful symlink farm manager

Symsync is a simple, modern symlink farm manager and an alternative to [GNU Stow](https://www.gnu.org/software/stow/). It aims to fill the gap between stow and more advanced tools like [chezmoi](https://www.chezmoi.io/). Symsync uses only live symlinks, so you can edit a file in the repo or from the app that uses it, and both stay the same.

Each target links to a state link in a machine-local state directory, and the state link links to the source in your repo. This makes every managed link self-identifying, lets symsync safely remove links it created, and lets a repo move without changes to the links in your home directory. Links are grouped into named packages, and one machine can use packages from many repos.

See the [design document](docs/design.md) for the full design.

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

Wildcards do not match names that start with `.`, so `**/*` does not go into `.git/`. A literal `.` in a glob matches them, as in `.*` above. To let wildcards match hidden files too, set `hidden = true` on the link group.

## Usage

The examples below use the config above, in a repo at `~/dotfiles`.

### Plan and sync

`plan` shows the changes without making them. `sync` prints the same lines as it applies them. A later `sync` with nothing to change prints `already in sync`.

```
$ symsync plan
mkdir   ~/.config/zed
link    ~/.config/zed/settings.json -> ~/dotfiles/zed/settings.json
mkdir   ~/.config/zed/themes
link    ~/.config/zed/themes/dark.json -> ~/dotfiles/zed/themes/dark.json
link    ~/.zshenv -> ~/dotfiles/zsh/.zshenv
link    ~/.zshrc -> ~/dotfiles/zsh/.zshrc

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

### Config and schema

`config` prints the parsed config as JSON, and `gen-config-schema` prints the JSON schema of the config format.
