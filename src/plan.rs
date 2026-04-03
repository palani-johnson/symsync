use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::config::SymsyncSyncConfig;

/// Configuration for a single state item — one source file to be linked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateItem {
    /// The target directory the symlink will be created in.
    pub link_path: String,
    /// The relative source path, used as the symlink filename within `link_path`.
    pub link: String,
    /// SHA-1 of `link`, used as the unique stem inside the state directory.
    pub state_file_hash: String,
    /// Group names this item belongs to.
    #[serde(default)]
    pub groups: HashSet<String>,
}

impl StateItem {
    /// Generate state items by expanding all glob sources in `config`, rooted at `root_dir`.
    pub fn generate(root_dir: &Path, config: &SymsyncSyncConfig) -> Vec<Self> {
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
                    let state_file_hash = format!("{:x}", hasher.finalize());

                    items.push(StateItem {
                        link_path: target.to_string_lossy().to_string(),
                        link: rel_str,
                        state_file_hash,
                        groups: entry.groups.clone(),
                    });
                }
            }
        }

        items
    }
}

/// A single planned step to execute.
#[derive(Debug)]
pub enum PlanStep {
    /// Nothing to do — the desired state already exists.
    NoOp { message: String },
    /// An unresolvable conflict that blocks execution.
    Error { message: String },
    /// Create a symlink at `symlink_path` pointing to `target_path`.
    Symlink {
        message: String,
        symlink_path: PathBuf,
        target_path: PathBuf,
    },
    /// Create a directory (and all parents) at `path`.
    Mkdir { message: String, path: PathBuf },
}

impl PlanStep {
    pub fn message(&self) -> &str {
        match self {
            PlanStep::NoOp { message }
            | PlanStep::Error { message }
            | PlanStep::Symlink { message, .. }
            | PlanStep::Mkdir { message, .. } => message,
        }
    }

    pub fn execute(&self) -> Result<()> {
        match self {
            PlanStep::NoOp { .. } => Ok(()),
            PlanStep::Error { message } => bail!("{}", message),
            PlanStep::Symlink {
                symlink_path,
                target_path,
                ..
            } => {
                symlink::symlink_file(target_path, symlink_path)?;
                Ok(())
            }
            PlanStep::Mkdir { path, .. } => {
                std::fs::create_dir_all(path)?;
                Ok(())
            }
        }
    }
}

/// Build a plan for linking a single state item through the state directory.
///
/// The two-hop chain this produces is:
///
///   `link_path/link`  →  `state_dir/<hash>.source`  →  `../../true_path`
///
/// A companion index link is also planned:
///
///   `state_dir/<hash>.target`  →  `/absolute/link_path/link`
///
/// This makes every managed file discoverable from `.symsync.state` alone:
/// - `.source` tells you what file is being managed.
/// - `.target` tells you where it has been linked to.
pub fn plan_link(state_item: &StateItem, state_dir: &Path) -> Vec<PlanStep> {
    let mut steps = Vec::new();

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

    // Absolute path of the user-visible link (may not exist yet).
    let link_path_abs = if link_path.is_absolute() {
        link_path.clone()
    } else {
        std::env::current_dir().unwrap_or_default().join(&link_path)
    };

    // Paths of the two state entries.
    let source_path = state_dir.join(format!("{}.source", state_item.state_file_hash));
    let target_path = state_dir.join(format!("{}.target", state_item.state_file_hash));

    // Relative path from `state_dir` (the parent of the .source symlink) to
    // the real source file. The OS resolves relative symlink targets from the
    // directory containing the symlink, so `state_dir` is the correct base.
    let source_to_true =
        pathdiff::diff_paths(&true_path, &state_dir).unwrap_or_else(|| true_path.clone());

    // --- Step 1: state_dir/<hash>.source → true_path (relative) ---
    if source_path.is_symlink() {
        match std::fs::read_link(&source_path) {
            Ok(existing) if existing == source_to_true => {
                steps.push(PlanStep::NoOp {
                    message: format!(
                        "{} already points to {}",
                        source_path.display(),
                        source_to_true.display()
                    ),
                });
            }
            _ => {
                steps.push(PlanStep::Error {
                    message: format!(
                        "{} exists but points to an invalid location",
                        source_path.display()
                    ),
                });
            }
        }
    } else if source_path.exists() {
        steps.push(PlanStep::Error {
            message: format!("{} exists but is not a symlink", source_path.display()),
        });
    } else {
        steps.push(PlanStep::Symlink {
            message: format!("{} → {}", source_path.display(), source_to_true.display()),
            symlink_path: source_path.clone(),
            target_path: source_to_true,
        });
    }

    // --- Step 2: state_dir/<hash>.target → link_path (absolute) ---
    if target_path.is_symlink() {
        match std::fs::read_link(&target_path) {
            Ok(existing) if existing == link_path_abs => {
                steps.push(PlanStep::NoOp {
                    message: format!(
                        "{} already points to {}",
                        target_path.display(),
                        link_path_abs.display()
                    ),
                });
            }
            _ => {
                steps.push(PlanStep::Error {
                    message: format!(
                        "{} exists but points to an invalid location",
                        target_path.display()
                    ),
                });
            }
        }
    } else if target_path.exists() {
        steps.push(PlanStep::Error {
            message: format!("{} exists but is not a symlink", target_path.display()),
        });
    } else {
        steps.push(PlanStep::Symlink {
            message: format!("{} → {}", target_path.display(), link_path_abs.display()),
            symlink_path: target_path,
            target_path: link_path_abs.clone(),
        });
    }

    // --- Step 3: ensure link_path's parent directory exists ---
    if let Some(parent) = link_path.parent() {
        if !parent.as_os_str().is_empty() {
            if parent.exists() {
                if parent.is_dir() {
                    steps.push(PlanStep::NoOp {
                        message: format!(
                            "Link parent directory {} already exists",
                            parent.display()
                        ),
                    });
                } else {
                    steps.push(PlanStep::Error {
                        message: format!(
                            "Link parent directory {} exists but is not a directory",
                            parent.display()
                        ),
                    });
                }
            } else {
                steps.push(PlanStep::Mkdir {
                    message: format!("Link parent directory {} created", parent.display()),
                    path: parent.to_path_buf(),
                });
            }
        }
    }

    // --- Step 4: link_path → state_dir/<hash>.source (absolute) ---
    if link_path.is_symlink() {
        match std::fs::read_link(&link_path) {
            Ok(existing) if existing == source_path => {
                steps.push(PlanStep::NoOp {
                    message: format!(
                        "{} already points to {}",
                        link_path.display(),
                        source_path.display()
                    ),
                });
            }
            _ => {
                steps.push(PlanStep::Error {
                    message: format!(
                        "{} exists but points to an invalid location",
                        link_path.display()
                    ),
                });
            }
        }
    } else if link_path.exists() {
        steps.push(PlanStep::Error {
            message: format!("{} exists but is not a symlink", link_path.display()),
        });
    } else {
        steps.push(PlanStep::Symlink {
            message: format!("{} → {}", link_path.display(), source_path.display()),
            symlink_path: link_path,
            target_path: source_path,
        });
    }

    steps
}
