use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a single sync operation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SyncEntry {
    /// Globs used to find source files for the sync.
    pub sources: Vec<String>,

    /// Target directory for the sync.
    pub target: String,

    /// A set of group names this sync entry belongs to.
    #[serde(default)]
    pub groups: HashSet<String>,
}

/// Top-level symsync configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SymsyncSyncConfig {
    /// A sequence of sync entries.
    pub entries: Vec<SyncEntry>,
}

impl SymsyncSyncConfig {
    /// Parse the config file at the given path.
    ///
    /// Supports `.toml`, `.json`, `.json5`, `.yaml`, and `.yml` extensions.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The path does not exist or is not a file
    /// - The file cannot be read
    /// - The file extension is not supported
    /// - The file contents are not valid for the detected format
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

            Some("json") | Some("json5") => json5::from_str(&content)
                .with_context(|| format!("Invalid JSON in {}", path.display())),

            Some("yaml") | Some("yml") => serde_yaml::from_str(&content)
                .with_context(|| format!("Invalid YAML in {}", path.display())),

            Some(ext) => bail!("Unsupported config file extension: .{}", ext),

            None => bail!("Config file has no extension"),
        }
    }
}
