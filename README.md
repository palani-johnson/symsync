# Symsync - A stateful symlink farm manager

Symsync is a simple, modern symlink farm manager and an alternative to [GNU Stow](https://www.gnu.org/software/stow/). It aims to fill the gap between stow and more advanced tools like [chezmoi](https://www.chezmoi.io/). Symsync uses only live symlinks, so you can edit a file in the repo or from the app that uses it, and both stay the same.

Each target links to a state link in a machine-local state directory, and the state link links to the source in your repo. This makes every managed link self-identifying, lets symsync safely remove links it created, and lets a repo move without changes to the links in your home directory. Links are grouped into named packages, and one machine can use packages from many repos.

See the [design document](docs/design.md) for the full design.
