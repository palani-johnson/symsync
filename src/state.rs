use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::LinkType;
use crate::paths::{normalize, relative, state_link_name};

const VERSION: u32 = 1;
const PACKAGES_FILE: &str = "packages.toml";
const LINKS_DIR: &str = "links";
const LOCK_FILE: &str = "lock";

/// A package as recorded in the state. All paths are absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRecord {
    pub config: PathBuf,
    pub link_type: LinkType,
    /// Source of each target, keyed by target.
    pub targets: BTreeMap<PathBuf, PathBuf>,
}

/// The state directory, held under a lock while open.
#[derive(Debug)]
pub struct State {
    dir: PathBuf,
    _lock: Option<File>,
    pub packages: BTreeMap<String, PackageRecord>,
}

impl State {
    /// Opens the state for reading without changes to the file system. A missing state directory gives an empty state.
    pub fn read(dir: &Path) -> Result<Self> {
        if !dir.exists() {
            return Ok(Self {
                dir: dir.to_path_buf(),
                _lock: None,
                packages: BTreeMap::new(),
            });
        }
        check_layout(dir)?;

        let lock = open_lock(dir)?;
        lock.lock_shared()
            .with_context(|| format!("failed to lock {}", dir.display()))?;

        Ok(Self {
            dir: dir.to_path_buf(),
            packages: read_packages(dir)?,
            _lock: Some(lock),
        })
    }

    /// Opens the state for changes. Creates the state directory if necessary.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir.join(LINKS_DIR))
            .with_context(|| format!("failed to create {}", dir.display()))?;
        check_layout(dir)?;

        let lock = open_lock(dir)?;
        lock.lock()
            .with_context(|| format!("failed to lock {}", dir.display()))?;

        Ok(Self {
            dir: dir.to_path_buf(),
            packages: read_packages(dir)?,
            _lock: Some(lock),
        })
    }

    pub fn links_dir(&self) -> PathBuf {
        self.dir.join(LINKS_DIR)
    }

    /// Path of the state link for a target.
    ///
    /// The name comes from the target path relative to the state directory, so it stays the same when a relative repo and its state directory move together.
    pub fn link_path(&self, target: &Path) -> PathBuf {
        self.links_dir()
            .join(state_link_name(&relative(target, &self.dir)))
    }

    /// Entries in the links directory that no package records.
    pub fn stray_links(&self) -> Result<Vec<PathBuf>> {
        let links_dir = self.links_dir();
        if !links_dir.exists() {
            return Ok(Vec::new());
        }

        let recorded: HashSet<PathBuf> = self
            .packages
            .values()
            .flat_map(|record| record.targets.keys())
            .map(|target| self.link_path(target))
            .collect();

        let mut stray = Vec::new();
        for entry in std::fs::read_dir(&links_dir)
            .with_context(|| format!("failed to read {}", links_dir.display()))?
        {
            let path = entry?.path();
            if !recorded.contains(&path) {
                stray.push(path);
            }
        }
        stray.sort();
        Ok(stray)
    }

    /// Writes the packages file atomically.
    pub fn save(&self) -> Result<()> {
        let file = PackagesFile {
            version: VERSION,
            packages: self
                .packages
                .iter()
                .map(|(name, record)| (name.clone(), to_stored(record, &self.dir)))
                .collect(),
        };
        let content = toml::to_string_pretty(&file).context("failed to serialize packages")?;

        let path = self.dir.join(PACKAGES_FILE);
        let tmp = self.dir.join(format!("{PACKAGES_FILE}.tmp"));
        std::fs::write(&tmp, content)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    }
}

fn check_layout(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        bail!("state directory {} is not a directory", dir.display());
    }
    let links = dir.join(LINKS_DIR);
    if links.exists() && !links.is_dir() {
        bail!("{} is not a directory", links.display());
    }
    Ok(())
}

fn open_lock(dir: &Path) -> Result<File> {
    let path = dir.join(LOCK_FILE);
    File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))
}

#[derive(Debug, Serialize, Deserialize)]
struct PackagesFile {
    version: u32,
    #[serde(default)]
    packages: BTreeMap<String, StoredPackage>,
}

/// A package as written to disk. For relative packages, paths are relative to the state directory.
#[derive(Debug, Serialize, Deserialize)]
struct StoredPackage {
    config: PathBuf,
    link_type: LinkType,
    #[serde(default)]
    targets: BTreeMap<PathBuf, StoredTarget>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredTarget {
    source: PathBuf,
}

fn read_packages(dir: &Path) -> Result<BTreeMap<String, PackageRecord>> {
    let path = dir.join(PACKAGES_FILE);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file: PackagesFile =
        toml::from_str(&content).with_context(|| format!("invalid {}", path.display()))?;
    if file.version != VERSION {
        bail!(
            "unsupported version {} in {}, expected {VERSION}",
            file.version,
            path.display()
        );
    }

    Ok(file
        .packages
        .into_iter()
        .map(|(name, stored)| (name, from_stored(stored, dir)))
        .collect())
}

fn to_stored(record: &PackageRecord, dir: &Path) -> StoredPackage {
    let store = |path: &Path| match record.link_type {
        LinkType::Absolute => path.to_path_buf(),
        LinkType::Relative => pathdiff::diff_paths(path, dir).unwrap_or_else(|| path.to_path_buf()),
    };

    StoredPackage {
        config: store(&record.config),
        link_type: record.link_type,
        targets: record
            .targets
            .iter()
            .map(|(target, source)| {
                (
                    store(target),
                    StoredTarget {
                        source: store(source),
                    },
                )
            })
            .collect(),
    }
}

fn from_stored(stored: StoredPackage, dir: &Path) -> PackageRecord {
    let load = |path: PathBuf| normalize(&dir.join(path));

    PackageRecord {
        config: load(stored.config),
        link_type: stored.link_type,
        targets: stored
            .targets
            .into_iter()
            .map(|(target, stored)| (load(target), load(stored.source)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(root: &Path, link_type: LinkType) -> PackageRecord {
        PackageRecord {
            config: root.join("repo/symsync.toml"),
            link_type,
            targets: BTreeMap::from([(root.join("home/.zshrc"), root.join("repo/zsh/.zshrc"))]),
        }
    }

    #[test]
    fn read_of_missing_dir_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state");

        let state = State::read(&dir).unwrap();
        assert!(state.packages.is_empty());
        assert!(!dir.exists());
    }

    #[test]
    fn open_creates_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state");

        State::open(&dir).unwrap();
        assert!(dir.join(LINKS_DIR).is_dir());
        assert!(dir.join(LOCK_FILE).is_file());
    }

    #[test]
    fn round_trips_absolute() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state");
        let expected = record(tmp.path(), LinkType::Absolute);

        let mut state = State::open(&dir).unwrap();
        state.packages.insert("zsh".into(), expected.clone());
        state.save().unwrap();
        drop(state);

        assert_eq!(State::read(&dir).unwrap().packages["zsh"], expected);
    }

    #[test]
    fn round_trips_relative_with_relative_paths_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state");
        let expected = record(tmp.path(), LinkType::Relative);

        let mut state = State::open(&dir).unwrap();
        state.packages.insert("zsh".into(), expected.clone());
        state.save().unwrap();
        drop(state);

        let content = std::fs::read_to_string(dir.join(PACKAGES_FILE)).unwrap();
        assert!(!content.contains(tmp.path().to_str().unwrap()));
        assert!(content.contains("../repo/symsync.toml"));

        assert_eq!(State::read(&dir).unwrap().packages["zsh"], expected);
    }

    #[test]
    fn rejects_unknown_version() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(PACKAGES_FILE), "version = 2").unwrap();
        assert!(State::read(tmp.path()).is_err());
    }

    #[test]
    fn link_path_hashes_target_relative_to_state_dir() {
        let state = State::read(Path::new("/state")).unwrap();

        assert_eq!(
            state.link_path(Path::new("/home/user/.zshrc")),
            Path::new("/state/links").join(state_link_name(Path::new("../home/user/.zshrc")))
        );
    }

    #[test]
    fn link_path_is_stable_when_repo_and_state_move_together() {
        let before = State::read(Path::new("/a/repo/.state")).unwrap();
        let after = State::read(Path::new("/b/copy/.state")).unwrap();

        assert_eq!(
            before.link_path(Path::new("/a/repo/out/x")).file_name(),
            after.link_path(Path::new("/b/copy/out/x")).file_name()
        );
    }

    #[test]
    fn finds_stray_links() {
        let tmp = tempfile::tempdir().unwrap();
        let mut state = State::open(&tmp.path().join("state")).unwrap();
        let expected = record(tmp.path(), LinkType::Absolute);
        let target = expected.targets.keys().next().unwrap().clone();
        state.packages.insert("zsh".into(), expected);

        let recorded = state.link_path(&target);
        let stray = state.links_dir().join("stray");
        std::os::unix::fs::symlink("/x", &recorded).unwrap();
        std::os::unix::fs::symlink("/x", &stray).unwrap();

        assert_eq!(state.stray_links().unwrap(), [stray]);
    }
}
