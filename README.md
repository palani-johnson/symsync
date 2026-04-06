# Symsync - A stateful symlink farm manager

Symsymc is a simple modern symlink farm manager and alternative to [GNU Stow](https://www.gnu.org/software/stow/). It aims to bridge the gap between stow and more advanced tools like [chezmoi](https://www.chezmoi.io/). At it's core, symsync is a stateful symlink farm manager that is configured via of a `symsync.toml` configuration file (json, json5, jsonc, and yaml also supported).

## Stateful

Symsync does not simply link from the target path to the source. Instead, it creates a `.symsync.state` directory in the same directory as the config file. Targets point via absolute path to symlinks in the state directory which then point via relative path to the source. Links in the state directory are named via the sha1 hash of the relative path source. This doubly linked structure helps ensure that links managed by symsync are traceable. If you see a symlink that points to a file that lives in a `.symsync.state/sources` directory, you know it is managed by symsync. Additionally, a file called `targets.toml` is created which helps track target state . Here's a more detailed breakdown of the system:

| Name      | Path                                                  | Links to                |
| --------- | ----------------------------------------------------- | ----------------------- |
| Source    | relative to the directory where `symsync.toml` exists |                         |
| Target    | anywhere on the system                                | full path to StateLink  |
| StateLink | `.symsync.state/sources/{source_path_sha1}`           | relative path to Source |
| StateFile | `.symsync.state/targets.toml`                         |                         |

## Configuration

The configuration file `symsync.{toml,json,jsonc,json5,yaml,yml}` must live in the same directory that you plan to have the state directory live in. A json schema spec for the file can be found [here](schema/config.json) or generated via the `symsync gen-config-schema` command. Example configurations can be found in this repo's `example/` folder.
