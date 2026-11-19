use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const FILE_NAMES: [&str; 6] = [
    "symsync.toml",
    "symsync.json",
    "symsync.jsonc",
    "symsync.json5",
    "symsync.yaml",
    "symsync.yml",
];

/// How symsync stores every path of a package.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkType {
    /// All links and recorded paths are absolute.
    #[default]
    Absolute,

    /// All links and recorded paths are relative, so the chain works after the repo and state directory move together.
    Relative,
}

/// A symsync config file.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// State directory, relative to the config file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<PathBuf>,

    /// Link type for all packages in this config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_type: Option<LinkType>,

    /// Packages, keyed by name. Names must be unique in a state directory.
    pub packages: BTreeMap<String, Package>,
}

/// A named group of links.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Package {
    /// Link groups of the package.
    pub links: Vec<LinkGroup>,
}

/// A set of source files that link into one target directory.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkGroup {
    /// Directory that the source globs are relative to, relative to the config file. Defaults to the config directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,

    /// Globs that select source files.
    pub sources: Vec<String>,

    /// Target directory. Each match goes here, at its path relative to `root`.
    pub target: String,

    /// Let wildcards match names that start with `.`. Without this, only a literal `.` in a glob matches them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A config together with the absolute path of the file it came from.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: Config,
}

impl LoadedConfig {
    /// Loads the config at `path`, or finds the config in `dir` when `path` is `None`.
    pub fn load(path: Option<&Path>, dir: &Path) -> Result<Self> {
        let path = match path {
            Some(path) => path.to_path_buf(),
            None => find(dir)?,
        };
        let path = std::path::absolute(&path)
            .with_context(|| format!("failed to resolve {}", path.display()))?;
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config = parse(&content, &path)?;

        Ok(Self { path, config })
    }

    /// Like [`Self::load`], but gives `None` when `path` is `None` and `dir` has no config file.
    pub fn load_optional(path: Option<&Path>, dir: &Path) -> Result<Option<Self>> {
        if path.is_none() && !FILE_NAMES.iter().any(|name| dir.join(name).is_file()) {
            return Ok(None);
        }
        Self::load(path, dir).map(Some)
    }

    /// Directory that holds the config file.
    pub fn dir(&self) -> &Path {
        self.path
            .parent()
            .expect("config path is an absolute file path")
    }
}

fn find(dir: &Path) -> Result<PathBuf> {
    let found: Vec<PathBuf> = FILE_NAMES
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .collect();

    match found.as_slice() {
        [path] => Ok(path.clone()),
        [] => bail!(
            "no config file found in {}, expected one of: {}",
            dir.display(),
            FILE_NAMES.join(", ")
        ),
        _ => bail!(
            "more than one config file found in {}: {}",
            dir.display(),
            found
                .iter()
                .filter_map(|p| p.file_name()?.to_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Parses `content` in the format given by the extension of `path`.
fn parse(content: &str, path: &Path) -> Result<Config> {
    let context = || format!("invalid config in {}", path.display());

    match path.extension().and_then(|e| e.to_str()) {
        Some("toml") => toml::from_str(content).with_context(context),
        Some("json" | "jsonc" | "json5") => json5::from_str(content).with_context(context),
        Some("yaml" | "yml") => serde_yaml_ng::from_str(content).with_context(context),
        _ => bail!("unsupported config file extension: {}", path.display()),
    }
}

/// JSON schema of the config format.
pub fn schema() -> schemars::Schema {
    schemars::schema_for!(Config)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = r#"
        state_dir = ".state"
        link_type = "relative"

        [[packages.zsh.links]]
        root = "zsh"
        sources = [".zshrc"]
        target = "~"
    "#;

    fn assert_example(config: &Config) {
        assert_eq!(config.state_dir.as_deref(), Some(Path::new(".state")));
        assert_eq!(config.link_type, Some(LinkType::Relative));
        let links = &config.packages["zsh"].links;
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].root.as_deref(), Some(Path::new("zsh")));
        assert_eq!(links[0].sources, [".zshrc"]);
        assert_eq!(links[0].target, "~");
    }

    #[test]
    fn parses_toml() {
        assert_example(&parse(TOML, Path::new("symsync.toml")).unwrap());
    }

    #[test]
    fn parses_json5() {
        let content = r#"{
            // comment
            state_dir: ".state",
            link_type: "relative",
            packages: { zsh: { links: [{ root: "zsh", sources: [".zshrc"], target: "~" }] } },
        }"#;
        assert_example(&parse(content, Path::new("symsync.jsonc")).unwrap());
    }

    #[test]
    fn parses_yaml() {
        let content = r#"
state_dir: .state
link_type: relative
packages:
  zsh:
    links:
      - root: zsh
        sources: [.zshrc]
        target: "~"
"#;
        assert_example(&parse(content, Path::new("symsync.yaml")).unwrap());
    }

    #[test]
    fn optional_fields_default() {
        let config = parse(
            "[[packages.a.links]]\nsources = [\"*\"]\ntarget = \"~\"",
            Path::new("symsync.toml"),
        )
        .unwrap();
        assert_eq!(config.state_dir, None);
        assert_eq!(config.link_type, None);
        assert_eq!(config.packages["a"].links[0].root, None);
    }

    #[test]
    fn rejects_unknown_fields() {
        let content = "[[packages.a.links]]\nsources = []\ntarget = \"~\"\nlink_typ = \"relative\"";
        assert!(parse(content, Path::new("symsync.toml")).is_err());
    }

    #[test]
    fn rejects_unknown_extension() {
        assert!(parse(TOML, Path::new("symsync.ini")).is_err());
    }

    #[test]
    fn finds_single_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("symsync.toml"), TOML).unwrap();

        let loaded = LoadedConfig::load(None, dir.path()).unwrap();
        assert_eq!(loaded.path, dir.path().join("symsync.toml"));
        assert_eq!(loaded.dir(), dir.path());
    }

    #[test]
    fn rejects_missing_config() {
        let dir = tempfile::tempdir().unwrap();
        assert!(LoadedConfig::load(None, dir.path()).is_err());
    }

    #[test]
    fn rejects_ambiguous_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("symsync.toml"), TOML).unwrap();
        std::fs::write(dir.path().join("symsync.json"), "{}").unwrap();
        assert!(LoadedConfig::load(None, dir.path()).is_err());
    }
}
