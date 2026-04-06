use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::config::{SymsyncConfig, TargetLinkType};

// ─── StateItem ────────────────────────────────────────────────────────────────

/// Configuration for a single state item — one source file to be linked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateItem {
    /// The target directory the symlink will be created in (absolute).
    pub link_path: String,
    /// The raw target string as written in the config (e.g. `"out"` or `"~/.config"`).
    pub target: String,
    /// The relative source path, used as the symlink filename within `link_path`.
    pub link: String,
    /// SHA-1 of `link`, used as the unique stem inside the state directory.
    pub state_hash: String,
    /// Whether the user-visible link should point to the state link via an
    /// absolute path or a relative path.
    pub target_link_type: TargetLinkType,
    /// Group names this item belongs to.
    #[serde(default)]
    pub groups: HashSet<String>,
}

impl StateItem {
    /// Generate state items by expanding all glob sources in `config`, rooted at `root_dir`.
    pub fn generate(root_dir: &Path, config: &SymsyncConfig) -> Vec<Self> {
        let mut items = Vec::new();

        for entry in &config.entries {
            let target = {
                let t = PathBuf::from(&entry.target);
                if t.is_absolute() {
                    t
                } else {
                    std::env::current_dir().unwrap_or_default().join(t)
                }
            };

            for source_glob in &entry.sources {
                let pattern = root_dir.join(source_glob).to_string_lossy().to_string();

                let paths = match glob::glob(&pattern) {
                    Ok(paths) => paths,
                    Err(_) => continue,
                };

                for path in paths.flatten() {
                    if !path.is_file() {
                        continue;
                    }

                    let Ok(rel) = path.strip_prefix(root_dir) else {
                        continue;
                    };

                    let rel_str = rel.to_string_lossy().to_string();

                    let mut hasher = Sha1::new();
                    hasher.update(rel_str.as_bytes());
                    let state_hash = format!("{:x}", hasher.finalize());

                    items.push(StateItem {
                        link_path: target.to_string_lossy().to_string(),
                        target: entry.target.clone(),
                        link: rel_str,
                        state_hash,
                        target_link_type: entry.target_link_type.clone(),
                        groups: entry.groups.clone(),
                    });
                }
            }
        }

        items
    }
}

// ─── PlanStep trait ───────────────────────────────────────────────────────────

/// A single step in a sync plan.
pub trait PlanStep {
    /// A human-readable description of what this step will do.
    fn message(&self) -> &str;

    /// Execute this step.
    fn execute(&self) -> Result<()>;
}

// ─── Concrete step types ──────────────────────────────────────────────────────

/// A step that does nothing — the desired state already exists.
pub struct NoOp {
    pub message: String,
}

/// A step that represents an unresolvable conflict and blocks execution.
pub struct ErrorStep {
    pub message: String,
}

/// A step that creates a symlink at `symlink_path` pointing to `target_path`.
pub struct SymlinkStep {
    pub message: String,
    pub symlink_path: PathBuf,
    pub target_path: PathBuf,
}

/// A step that creates a directory (and all parents) at `path`.
pub struct MkdirStep {
    pub message: String,
    pub path: PathBuf,
}

// ─── PlanStep implementations ─────────────────────────────────────────────────

impl PlanStep for NoOp {
    fn message(&self) -> &str {
        &self.message
    }

    fn execute(&self) -> Result<()> {
        Ok(())
    }
}

impl PlanStep for ErrorStep {
    fn message(&self) -> &str {
        &self.message
    }

    fn execute(&self) -> Result<()> {
        bail!("{}", self.message)
    }
}

impl PlanStep for SymlinkStep {
    fn message(&self) -> &str {
        &self.message
    }

    fn execute(&self) -> Result<()> {
        symlink::symlink_file(&self.target_path, &self.symlink_path)?;
        Ok(())
    }
}

impl PlanStep for MkdirStep {
    fn message(&self) -> &str {
        &self.message
    }

    fn execute(&self) -> Result<()> {
        std::fs::create_dir_all(&self.path)?;
        Ok(())
    }
}

// ─── plan_link ────────────────────────────────────────────────────────────────

/// Build a plan for linking a single state item through the state directory.
///
/// The two-hop chain this produces is:
///
///   `link_path/link`  →  `state_dir/sources/<hash>`  →  `../../true_path`
///
/// Target symlinks point via absolute path into `.symsync.state/sources/`, making
/// every symsync-managed symlink identifiable by its destination directory.
pub fn plan_link(state_item: &StateItem, state_dir: &Path) -> Vec<Box<dyn PlanStep>> {
    let mut steps: Vec<Box<dyn PlanStep>> = Vec::new();

    // The final symlink visible to the user.
    let link_path = PathBuf::from(&state_item.link_path).join(&state_item.link);

    // Absolute path of the real source file.
    let true_path = {
        let p = PathBuf::from(&state_item.link);
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir().unwrap_or_default().join(p)
        }
    };

    // Ensure state_dir is absolute so all derived paths are unambiguous.
    let state_dir = if state_dir.is_absolute() {
        state_dir.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(state_dir)
    };

    // .symsync.state/sources/ — the subdirectory all state links live in.
    let sources_dir = state_dir.join("sources");

    // The state link for this item: .symsync.state/sources/<sha1>
    let source_path = sources_dir.join(&state_item.state_hash);

    // Relative path from `sources_dir` (the directory containing the state link)
    // to the real source file. The OS resolves relative symlink targets from the
    // directory containing the symlink, so `sources_dir` is the correct base.
    let source_to_true =
        pathdiff::diff_paths(&true_path, &sources_dir).unwrap_or_else(|| true_path.clone());

    // --- Step 1: sources/<hash> → true_path (relative) ---
    if source_path.is_symlink() {
        match std::fs::read_link(&source_path) {
            Ok(existing) if existing == source_to_true => {
                steps.push(Box::new(NoOp {
                    message: format!(
                        "{} already points to {}",
                        source_path.display(),
                        source_to_true.display()
                    ),
                }));
            }
            _ => {
                steps.push(Box::new(ErrorStep {
                    message: format!(
                        "{} exists but points to an invalid location",
                        source_path.display()
                    ),
                }));
            }
        }
    } else if source_path.exists() {
        steps.push(Box::new(ErrorStep {
            message: format!("{} exists but is not a symlink", source_path.display()),
        }));
    } else {
        steps.push(Box::new(SymlinkStep {
            message: format!("{} → {}", source_path.display(), source_to_true.display()),
            symlink_path: source_path.clone(),
            target_path: source_to_true,
        }));
    }

    // --- Step 2: ensure link_path's parent directory exists ---
    if let Some(parent) = link_path.parent() {
        if !parent.as_os_str().is_empty() {
            if parent.exists() {
                if parent.is_dir() {
                    steps.push(Box::new(NoOp {
                        message: format!(
                            "Link parent directory {} already exists",
                            parent.display()
                        ),
                    }));
                } else {
                    steps.push(Box::new(ErrorStep {
                        message: format!(
                            "Link parent directory {} exists but is not a directory",
                            parent.display()
                        ),
                    }));
                }
            } else {
                steps.push(Box::new(MkdirStep {
                    message: format!("Link parent directory {} created", parent.display()),
                    path: parent.to_path_buf(),
                }));
            }
        }
    }

    // --- Step 3: link_path → sources/<hash> (absolute or relative) ---

    // Compute the path the user-visible link should point to. For absolute
    // links this is the full path to the state link. For relative links it is
    // the path from the link's parent directory to the state link, which is
    // what the OS uses when resolving the symlink.
    let link_target = match state_item.target_link_type {
        TargetLinkType::Absolute => source_path.clone(),
        TargetLinkType::Relative => {
            let link_parent = link_path.parent().unwrap_or(Path::new("."));
            pathdiff::diff_paths(&source_path, link_parent).unwrap_or_else(|| source_path.clone())
        }
    };

    if link_path.is_symlink() {
        match std::fs::read_link(&link_path) {
            Ok(existing) if existing == link_target => {
                steps.push(Box::new(NoOp {
                    message: format!(
                        "{} already points to {}",
                        link_path.display(),
                        link_target.display()
                    ),
                }));
            }
            _ => {
                steps.push(Box::new(ErrorStep {
                    message: format!(
                        "{} exists but points to an invalid location",
                        link_path.display()
                    ),
                }));
            }
        }
    } else if link_path.exists() {
        steps.push(Box::new(ErrorStep {
            message: format!("{} exists but is not a symlink", link_path.display()),
        }));
    } else {
        steps.push(Box::new(SymlinkStep {
            message: format!("{} → {}", link_path.display(), link_target.display()),
            symlink_path: link_path,
            target_path: link_target,
        }));
    }

    steps
}
