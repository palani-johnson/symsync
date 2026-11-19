use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::LinkType;
use crate::desired::Targets;
use crate::paths::normalize;
use crate::state::{PackageRecord, State};

/// A single change to the file system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    CreateDir(PathBuf),
    CreateLink { path: PathBuf, value: PathBuf },
    ReplaceLink { path: PathBuf, value: PathBuf },
    RemoveLink(PathBuf),
}

/// An action, with the line to show for it. Actions on state links have no line of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub message: Option<String>,
    pub action: Action,
}

/// A problem that stops sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    Exists(PathBuf),
    Drifted(PathBuf),
    ForeignLink(PathBuf),
    OwnedBy { target: PathBuf, package: String },
    NotALink(PathBuf),
    ParentNotDir { target: PathBuf, parent: PathBuf },
    PackageElsewhere { package: String, config: PathBuf },
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exists(target) => write!(
                f,
                "{} exists and is not managed by symsync",
                target.display()
            ),
            Self::Drifted(target) => write!(
                f,
                "{} was replaced by a regular file, use `symsync adopt` to recover it",
                target.display()
            ),
            Self::ForeignLink(target) => write!(
                f,
                "{} is a symlink that symsync does not manage",
                target.display()
            ),
            Self::OwnedBy { target, package } => {
                write!(f, "{} is owned by package {package}", target.display())
            }
            Self::NotALink(path) => write!(f, "state link {} is not a symlink", path.display()),
            Self::ParentNotDir { target, parent } => write!(
                f,
                "cannot link {} because {} is not a directory",
                target.display(),
                parent.display()
            ),
            Self::PackageElsewhere { package, config } => {
                write!(f, "package {package} is owned by {}", config.display())
            }
        }
    }
}

/// The changes that make the file system and the state match a config.
#[derive(Debug, Default)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub conflicts: Vec<Conflict>,
    /// Information that does not stop sync.
    pub notes: Vec<String>,
    /// New record of each package that the plan changes. `None` removes the package.
    pub records: BTreeMap<String, Option<PackageRecord>>,
}

/// Plans the changes for the packages that `config` wants. Reads the file system but does not change it.
pub fn plan(
    state: &State,
    config: &Path,
    link_type: LinkType,
    desired: &BTreeMap<String, Targets>,
    home: &Path,
) -> Plan {
    let mut planner = Planner {
        state,
        config,
        link_type,
        home,
        plan: Plan::default(),
        removed: HashSet::new(),
        created_dirs: HashSet::new(),
    };

    // Removals come first, so that a target can move between packages in one sync.
    for (name, targets) in desired {
        if let Some(record) = state.packages.get(name)
            && record.config == config
        {
            for target in record.targets.keys() {
                if !targets.contains_key(target) {
                    planner.unlink(target);
                }
            }
        }
    }

    for (name, targets) in desired {
        let record = state.packages.get(name);
        if let Some(record) = record
            && record.config != config
        {
            planner.plan.conflicts.push(Conflict::PackageElsewhere {
                package: name.clone(),
                config: record.config.clone(),
            });
            continue;
        }

        for (target, source) in targets {
            planner.link(name, record, target, source);
        }

        planner.plan.records.insert(
            name.clone(),
            Some(PackageRecord {
                config: config.to_path_buf(),
                link_type,
                targets: targets.clone(),
            }),
        );
    }

    planner.plan
}

struct Planner<'a> {
    state: &'a State,
    config: &'a Path,
    link_type: LinkType,
    home: &'a Path,
    plan: Plan,
    /// Paths that earlier steps remove.
    removed: HashSet<PathBuf>,
    /// Directories that earlier steps create.
    created_dirs: HashSet<PathBuf>,
}

enum Entry {
    Missing,
    Link(PathBuf),
    File,
    Dir,
}

impl Planner<'_> {
    fn inspect(&self, path: &Path) -> Entry {
        if self.removed.contains(path) {
            return Entry::Missing;
        }
        match std::fs::symlink_metadata(path) {
            Err(_) => Entry::Missing,
            Ok(meta) if meta.file_type().is_symlink() => {
                Entry::Link(std::fs::read_link(path).unwrap_or_default())
            }
            Ok(meta) if meta.is_dir() => Entry::Dir,
            Ok(_) => Entry::File,
        }
    }

    fn show(&self, path: &Path) -> String {
        match path.strip_prefix(self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }

    fn push(&mut self, message: Option<String>, action: Action) {
        self.plan.steps.push(Step { message, action });
    }

    fn unlink(&mut self, target: &Path) {
        let link = self.state.link_path(target);

        match self.inspect(target) {
            Entry::Link(value) if resolves_to(&value, target, &link) => {
                self.push(
                    Some(format!("unlink  {}", self.show(target))),
                    Action::RemoveLink(target.to_path_buf()),
                );
                self.removed.insert(target.to_path_buf());
            }
            Entry::Missing => {}
            _ => self.plan.notes.push(format!(
                "{} is no longer managed, it was left in place because it is not a symsync link",
                self.show(target)
            )),
        }

        match self.inspect(&link) {
            Entry::Link(_) => {
                self.push(None, Action::RemoveLink(link.clone()));
                self.removed.insert(link);
            }
            Entry::Missing => {}
            _ => self.plan.conflicts.push(Conflict::NotALink(link)),
        }
    }

    fn link(&mut self, name: &str, record: Option<&PackageRecord>, target: &Path, source: &Path) {
        let owner = self.state.packages.iter().find(|(other, other_record)| {
            *other != name
                && other_record.config != self.config
                && other_record.targets.contains_key(target)
        });
        if let Some((owner, _)) = owner {
            self.plan.conflicts.push(Conflict::OwnedBy {
                target: target.to_path_buf(),
                package: owner.clone(),
            });
            return;
        }

        let link = self.state.link_path(target);
        let parent = target.parent().unwrap_or(Path::new("/"));
        let (state_value, target_value) = match self.link_type {
            LinkType::Absolute => (source.to_path_buf(), link.clone()),
            LinkType::Relative => (
                relative(source, &self.state.links_dir()),
                relative(&link, parent),
            ),
        };
        let was_managed = record.is_some_and(|record| record.targets.contains_key(target));

        let target_action = match self.inspect(target) {
            Entry::Missing => Some(Action::CreateLink {
                path: target.to_path_buf(),
                value: target_value,
            }),
            Entry::Link(value) if value == target_value => None,
            Entry::Link(value) if resolves_to(&value, target, &link) => Some(Action::ReplaceLink {
                path: target.to_path_buf(),
                value: target_value,
            }),
            Entry::Link(_) => return self.conflict(Conflict::ForeignLink(target.to_path_buf())),
            Entry::File if was_managed => {
                return self.conflict(Conflict::Drifted(target.to_path_buf()));
            }
            Entry::File | Entry::Dir => {
                return self.conflict(Conflict::Exists(target.to_path_buf()));
            }
        };

        let state_action = match self.inspect(&link) {
            Entry::Missing => Some(Action::CreateLink {
                path: link,
                value: state_value,
            }),
            Entry::Link(value) if value == state_value => None,
            Entry::Link(_) => Some(Action::ReplaceLink {
                path: link,
                value: state_value,
            }),
            Entry::File | Entry::Dir => return self.conflict(Conflict::NotALink(link)),
        };

        let creates_target = matches!(target_action, Some(Action::CreateLink { .. }));
        let dir_action = if creates_target {
            match self.parent_dir(target, parent) {
                Ok(action) => action,
                Err(conflict) => return self.conflict(conflict),
            }
        } else {
            None
        };

        let verb = if creates_target { "link  " } else { "relink" };
        let message = format!("{verb}  {} -> {}", self.show(target), self.show(source));

        match (state_action, target_action) {
            (None, None) => {}
            (Some(state_action), None) => self.push(Some(message), state_action),
            (state_action, Some(target_action)) => {
                if let Some(state_action) = state_action {
                    self.push(None, state_action);
                }
                if let Some(Action::CreateDir(dir)) = &dir_action {
                    let line = format!("mkdir   {}", self.show(dir));
                    self.push(Some(line), Action::CreateDir(dir.clone()));
                }
                self.push(Some(message), target_action);
            }
        }
    }

    /// Plans the creation of `parent`, if necessary.
    fn parent_dir(&mut self, target: &Path, parent: &Path) -> Result<Option<Action>, Conflict> {
        for ancestor in parent.ancestors() {
            if self.created_dirs.contains(ancestor) {
                break;
            }
            if let Ok(meta) = std::fs::metadata(ancestor) {
                if !meta.is_dir() {
                    return Err(Conflict::ParentNotDir {
                        target: target.to_path_buf(),
                        parent: ancestor.to_path_buf(),
                    });
                }
                break;
            }
        }

        if parent.is_dir() || self.created_dirs.contains(parent) {
            return Ok(None);
        }
        self.created_dirs
            .extend(parent.ancestors().map(Path::to_path_buf));
        Ok(Some(Action::CreateDir(parent.to_path_buf())))
    }

    fn conflict(&mut self, conflict: Conflict) {
        self.plan.conflicts.push(conflict);
    }
}

/// Whether the symlink at `at`, with the content `value`, points to `expected`.
fn resolves_to(value: &Path, at: &Path, expected: &Path) -> bool {
    let base = at.parent().unwrap_or(Path::new("/"));
    normalize(&base.join(value)) == expected
}

fn relative(path: &Path, from: &Path) -> PathBuf {
    pathdiff::diff_paths(path, from).unwrap_or_else(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        home: PathBuf,
        config: PathBuf,
        state: State,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(root.join("repo")).unwrap();
        std::fs::write(root.join("repo/a"), "a").unwrap();
        std::fs::write(root.join("repo/b"), "b").unwrap();
        let state = State::open(&root.join("state")).unwrap();
        Fixture {
            _tmp: tmp,
            config: root.join("repo/symsync.toml"),
            root,
            home,
            state,
        }
    }

    impl Fixture {
        fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }

        fn desired(&self, packages: &[(&str, &[(&str, &str)])]) -> BTreeMap<String, Targets> {
            packages
                .iter()
                .map(|(name, targets)| {
                    let targets = targets
                        .iter()
                        .map(|(target, source)| (self.path(target), self.path(source)))
                        .collect();
                    (name.to_string(), targets)
                })
                .collect()
        }

        fn plan(&self, link_type: LinkType, desired: &BTreeMap<String, Targets>) -> Plan {
            plan(&self.state, &self.config, link_type, desired, &self.home)
        }

        fn record(&mut self, name: &str, config: &Path, targets: &[(&str, &str)]) {
            let record = PackageRecord {
                config: config.to_path_buf(),
                link_type: LinkType::Absolute,
                targets: targets
                    .iter()
                    .map(|(target, source)| (self.path(target), self.path(source)))
                    .collect(),
            };
            self.state.packages.insert(name.into(), record);
        }

        /// Applies the plan and records its packages, as sync will.
        fn apply(&mut self, plan: Plan) {
            assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
            for step in plan.steps {
                match step.action {
                    Action::CreateDir(dir) => std::fs::create_dir_all(dir).unwrap(),
                    Action::CreateLink { path, value } => symlink(value, path).unwrap(),
                    Action::ReplaceLink { path, value } => {
                        std::fs::remove_file(&path).unwrap();
                        symlink(value, path).unwrap();
                    }
                    Action::RemoveLink(path) => std::fs::remove_file(path).unwrap(),
                }
            }
            for (name, record) in plan.records {
                match record {
                    Some(record) => self.state.packages.insert(name, record),
                    None => self.state.packages.remove(&name),
                };
            }
        }
    }

    fn messages(plan: &Plan) -> Vec<&str> {
        plan.steps
            .iter()
            .filter_map(|s| s.message.as_deref())
            .collect()
    }

    #[test]
    fn links_new_target_through_state_link() {
        let mut f = fixture();
        let desired = f.desired(&[("p", &[("home/.config/x/a", "repo/a")])]);
        let plan = f.plan(LinkType::Absolute, &desired);

        let link = f.state.link_path(&f.path("home/.config/x/a"));
        assert_eq!(
            plan.steps.iter().map(|s| &s.action).collect::<Vec<_>>(),
            [
                &Action::CreateLink {
                    path: link.clone(),
                    value: f.path("repo/a")
                },
                &Action::CreateDir(f.path("home/.config/x")),
                &Action::CreateLink {
                    path: f.path("home/.config/x/a"),
                    value: link
                },
            ]
        );
        assert_eq!(
            messages(&plan),
            [
                "mkdir   ~/.config/x".to_string(),
                format!("link    ~/.config/x/a -> {}", f.path("repo/a").display()),
            ]
        );
        assert!(plan.records["p"].is_some());

        f.apply(plan);
        assert_eq!(
            std::fs::read_to_string(f.path("home/.config/x/a")).unwrap(),
            "a"
        );
    }

    #[test]
    fn synced_package_has_no_steps() {
        let mut f = fixture();
        let desired = f.desired(&[("p", &[("home/a", "repo/a")])]);
        let plan = f.plan(LinkType::Absolute, &desired);
        f.apply(plan);

        let plan = f.plan(LinkType::Absolute, &desired);
        assert!(plan.steps.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn creates_shared_parent_dir_once() {
        let f = fixture();
        let desired = f.desired(&[("p", &[("home/x/a", "repo/a"), ("home/x/b", "repo/b")])]);
        let plan = f.plan(LinkType::Absolute, &desired);

        let dirs = plan
            .steps
            .iter()
            .filter(|s| matches!(s.action, Action::CreateDir(_)))
            .count();
        assert_eq!(dirs, 1);
    }

    #[test]
    fn relative_links_point_relatively() {
        let mut f = fixture();
        let desired = f.desired(&[("p", &[("home/a", "repo/a")])]);
        let plan = f.plan(LinkType::Relative, &desired);
        f.apply(plan);

        let link = f.state.link_path(&f.path("home/a"));
        let target_value = std::fs::read_link(f.path("home/a")).unwrap();
        let state_value = std::fs::read_link(&link).unwrap();
        assert!(target_value.is_relative());
        assert!(state_value.is_relative());
        assert_eq!(std::fs::read_to_string(f.path("home/a")).unwrap(), "a");
    }

    #[test]
    fn link_type_change_relinks() {
        let mut f = fixture();
        let desired = f.desired(&[("p", &[("home/a", "repo/a")])]);
        let plan = f.plan(LinkType::Absolute, &desired);
        f.apply(plan);

        let plan = f.plan(LinkType::Relative, &desired);
        assert!(matches!(plan.steps[0].action, Action::ReplaceLink { .. }));
        assert!(messages(&plan)[0].starts_with("relink  ~/a"));
        f.apply(plan);
        assert!(std::fs::read_link(f.path("home/a")).unwrap().is_relative());
    }

    #[test]
    fn changed_source_rewrites_only_state_link() {
        let mut f = fixture();
        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a")])]),
        );
        f.apply(plan);

        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/b")])]),
        );
        assert_eq!(
            plan.steps.iter().map(|s| &s.action).collect::<Vec<_>>(),
            [&Action::ReplaceLink {
                path: f.state.link_path(&f.path("home/a")),
                value: f.path("repo/b")
            }]
        );
        f.apply(plan);
        assert_eq!(std::fs::read_to_string(f.path("home/a")).unwrap(), "b");
    }

    #[test]
    fn removes_stale_target() {
        let mut f = fixture();
        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a"), ("home/b", "repo/b")])]),
        );
        f.apply(plan);

        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a")])]),
        );
        assert_eq!(messages(&plan), ["unlink  ~/b"]);
        let link = f.state.link_path(&f.path("home/b"));
        f.apply(plan);
        assert!(!f.path("home/b").exists());
        assert!(!link.is_symlink());
    }

    #[test]
    fn stale_target_that_is_not_a_link_is_left_in_place() {
        let mut f = fixture();
        f.record("p", &f.config.clone(), &[("home/b", "repo/b")]);
        std::fs::write(f.path("home/b"), "mine").unwrap();

        let plan = f.plan(LinkType::Absolute, &f.desired(&[("p", &[])]));
        assert!(plan.steps.is_empty());
        assert_eq!(plan.notes.len(), 1);
    }

    #[test]
    fn target_moves_between_packages() {
        let mut f = fixture();
        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a")]), ("q", &[])]),
        );
        f.apply(plan);

        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[]), ("q", &[("home/a", "repo/a")])]),
        );
        assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
        f.apply(plan);
        assert_eq!(std::fs::read_to_string(f.path("home/a")).unwrap(), "a");
        assert!(
            f.state.packages["q"]
                .targets
                .contains_key(&f.path("home/a"))
        );
    }

    #[test]
    fn unmanaged_file_is_a_conflict() {
        let f = fixture();
        std::fs::write(f.path("home/a"), "mine").unwrap();

        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a")])]),
        );
        assert_eq!(plan.conflicts, [Conflict::Exists(f.path("home/a"))]);
    }

    #[test]
    fn replaced_managed_link_is_drift() {
        let mut f = fixture();
        let desired = f.desired(&[("p", &[("home/a", "repo/a")])]);
        let plan = f.plan(LinkType::Absolute, &desired);
        f.apply(plan);
        std::fs::remove_file(f.path("home/a")).unwrap();
        std::fs::write(f.path("home/a"), "edited").unwrap();

        let plan = f.plan(LinkType::Absolute, &desired);
        assert_eq!(plan.conflicts, [Conflict::Drifted(f.path("home/a"))]);
    }

    #[test]
    fn foreign_link_is_a_conflict() {
        let f = fixture();
        symlink(f.path("repo/a"), f.path("home/a")).unwrap();

        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a")])]),
        );
        assert_eq!(plan.conflicts, [Conflict::ForeignLink(f.path("home/a"))]);
    }

    #[test]
    fn parent_that_is_a_file_is_a_conflict() {
        let f = fixture();
        std::fs::write(f.path("home/x"), "").unwrap();

        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/x/a", "repo/a")])]),
        );
        assert_eq!(
            plan.conflicts,
            [Conflict::ParentNotDir {
                target: f.path("home/x/a"),
                parent: f.path("home/x")
            }]
        );
    }

    #[test]
    fn target_of_package_in_other_config_is_a_conflict() {
        let mut f = fixture();
        f.record("q", &f.path("other/symsync.toml"), &[("home/a", "other/a")]);

        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a")])]),
        );
        assert_eq!(
            plan.conflicts,
            [Conflict::OwnedBy {
                target: f.path("home/a"),
                package: "q".into()
            }]
        );
    }

    #[test]
    fn package_of_other_config_is_a_conflict() {
        let mut f = fixture();
        f.record("p", &f.path("other/symsync.toml"), &[]);

        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a")])]),
        );
        assert_eq!(
            plan.conflicts,
            [Conflict::PackageElsewhere {
                package: "p".into(),
                config: f.path("other/symsync.toml")
            }]
        );
    }
}
