use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The link type for the target
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetLinkType {
    /// The link points to the state link using its full absolute path.
    Absolute,
    /// The link points to the state link using a path relative to the link's own parent directory.
    Relative,
}

/// Configuration for a single sync operation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SyncEntry {
    /// Globs used to find source files for the sync.
    pub sources: Vec<String>,

    /// Target directory for the sync.
    pub target: String,

    /// The link type for the target
    pub target_link_type: TargetLinkType,

    /// A set of group names this sync entry belongs to.
    #[serde(default)]
    pub groups: HashSet<String>,
}

/// Configuration for symsync
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SymsyncConfig {
    /// A sequence of sync entries.
    pub entries: Vec<SyncEntry>,
}

impl SymsyncConfig {
    /// Parse the config file at the given path.
    ///
    /// Supports `.toml`, `.json`, `.json5`, `.jsonc`, `.yaml`, and `.yml`.
    pub fn parse_file(path: &Path) -> Result<Self> {
        if !path.exists() {
            bail!("Config file not found: {}", path.display());
        }
        if !path.is_file() {
            bail!("Not a file: {}", path.display());
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        match path.extension().and_then(|e| e.to_str()) {
            Some("toml") => toml::from_str(&content)
                .with_context(|| format!("Invalid TOML in {}", path.display())),

            Some("json" | "json5" | "jsonc") => json5::from_str(&content)
                .with_context(|| format!("Invalid JSON in {}", path.display())),

            Some("yaml" | "yml") => serde_yaml::from_str(&content)
                .with_context(|| format!("Invalid YAML in {}", path.display())),

            Some(ext) => bail!("Unsupported config file extension: .{}", ext),

            None => bail!("Config file has no extension"),
        }
    }
}
