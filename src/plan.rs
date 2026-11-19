use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use crate::config::LinkType;
use crate::desired::Targets;
use crate::paths::{display, relative, resolves_to};
use crate::state::{PackageRecord, State};

/// A single change to the file system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    CreateDir(PathBuf),
    CreateLink {
        path: PathBuf,
        value: PathBuf,
    },
    ReplaceLink {
        path: PathBuf,
        value: PathBuf,
    },
    RemoveLink(PathBuf),
    /// Replaces the content of `to` with the regular file `from`, then removes `from`.
    MoveFile {
        from: PathBuf,
        to: PathBuf,
    },
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

impl Conflict {
    /// Text for output, with `~` for the home directory.
    pub fn message(&self, home: &Path) -> String {
        let show = |path: &Path| display(path, home);
        match self {
            Self::Exists(target) => {
                format!("{} exists and is not managed by symsync", show(target))
            }
            Self::Drifted(target) => format!(
                "{} was replaced by a regular file, use `symsync adopt` to recover it",
                show(target)
            ),
            Self::ForeignLink(target) => {
                format!("{} is a symlink that symsync does not manage", show(target))
            }
            Self::OwnedBy { target, package } => {
                format!("{} is owned by package {package}", show(target))
            }
            Self::NotALink(path) => format!("state link {} is not a symlink", show(path)),
            Self::ParentNotDir { target, parent } => format!(
                "cannot link {} because {} is not a directory",
                show(target),
                show(parent)
            ),
            Self::PackageElsewhere { package, config } => {
                format!("package {package} is owned by {}", show(config))
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
///
/// `still_defines` tells if the config at a path still defines a package. It must return `true` when it cannot tell. When `scope` is set, packages outside of it are left alone, even if the config no longer defines them.
pub fn plan(
    state: &State,
    config: &Path,
    link_type: LinkType,
    desired: &BTreeMap<String, Targets>,
    home: &Path,
    still_defines: &dyn Fn(&Path, &str) -> bool,
    scope: Option<&BTreeSet<String>>,
) -> Plan {
    let mut planner = Planner::new(state, config, link_type, home);

    let in_scope = |name: &String| scope.is_none_or(|scope| scope.contains(name));

    let mut owned = HashSet::new();
    for name in desired.keys().filter(|name| in_scope(name)) {
        match state.packages.get(name) {
            Some(record) if record.config == config => {}
            Some(record) if still_defines(&record.config, name) => {
                planner.plan.conflicts.push(Conflict::PackageElsewhere {
                    package: name.clone(),
                    config: record.config.clone(),
                });
                continue;
            }
            Some(record) => planner.plan.notes.push(format!(
                "package {name} moved from {} to {}",
                display(&record.config, home),
                display(config, home)
            )),
            None => {}
        }
        owned.insert(name);
    }

    // Removals come first, so that a target can move between packages in one sync.
    for (name, record) in &state.packages {
        match desired.get(name) {
            Some(targets) if owned.contains(name) => {
                for target in record.targets.keys() {
                    if !targets.contains_key(target) {
                        planner.unlink(target);
                    }
                }
            }
            None if record.config == config && in_scope(name) => {
                planner
                    .plan
                    .notes
                    .push(format!("package {name} was removed from the config"));
                for target in record.targets.keys() {
                    planner.unlink(target);
                }
                planner.plan.records.insert(name.clone(), None);
            }
            _ => {}
        }
    }

    for (name, targets) in desired {
        if !owned.contains(name) {
            continue;
        }
        let record = state.packages.get(name);

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

    planner.finish()
}

/// Plans the removal of a package and all its links, with the state as the only input.
pub fn plan_remove(state: &State, name: &str, record: &PackageRecord, home: &Path) -> Plan {
    let mut planner = Planner::new(state, &record.config, record.link_type, home);

    for target in record.targets.keys() {
        planner.unlink(target);
    }
    planner.plan.records.insert(name.to_string(), None);

    planner.finish()
}

/// Plans to move a drifted target back into its source and to link the target again.
pub fn plan_adopt(
    state: &State,
    name: &str,
    record: &PackageRecord,
    target: &Path,
    home: &Path,
) -> Plan {
    let mut planner = Planner::new(state, &record.config, record.link_type, home);
    let source = &record.targets[target];

    planner.push(
        Some(format!(
            "adopt   {} -> {}",
            planner.show(target),
            planner.show(source)
        )),
        Action::MoveFile {
            from: target.to_path_buf(),
            to: source.clone(),
        },
    );
    planner.removed.insert(target.to_path_buf());
    planner.link(name, Some(record), target, source);

    planner.finish()
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
    /// Targets that earlier steps unlink.
    unlinked: Vec<PathBuf>,
}

enum Entry {
    Missing,
    Link(PathBuf),
    File,
    Dir,
}

impl<'a> Planner<'a> {
    fn new(state: &'a State, config: &'a Path, link_type: LinkType, home: &'a Path) -> Self {
        Self {
            state,
            config,
            link_type,
            home,
            plan: Plan::default(),
            removed: HashSet::new(),
            created_dirs: HashSet::new(),
            unlinked: Vec::new(),
        }
    }

    /// Adds a note for each directory that the plan leaves without entries.
    fn finish(mut self) -> Plan {
        let linked: HashSet<&Path> = self
            .plan
            .steps
            .iter()
            .filter_map(|step| match &step.action {
                Action::CreateLink { path, .. } => path.parent(),
                _ => None,
            })
            .collect();
        let dirs: BTreeSet<&Path> = self.unlinked.iter().filter_map(|t| t.parent()).collect();

        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let empty = entries
                .flatten()
                .all(|entry| self.removed.contains(&entry.path()));
            if empty && !linked.contains(dir) {
                self.plan.notes.push(format!(
                    "{} has no entries left, symsync does not remove directories",
                    display(dir, self.home)
                ));
            }
        }
        self.plan
    }

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
        display(path, self.home)
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
                self.unlinked.push(target.to_path_buf());
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::execute::{run_steps, update_records};

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
            self.plan_with(link_type, desired, &|_, _| true)
        }

        fn plan_scoped(&self, desired: &BTreeMap<String, Targets>, scope: &[&str]) -> Plan {
            let scope = scope.iter().map(|s| s.to_string()).collect();
            plan(
                &self.state,
                &self.config,
                LinkType::Absolute,
                desired,
                &self.home,
                &|_, _| true,
                Some(&scope),
            )
        }

        fn plan_with(
            &self,
            link_type: LinkType,
            desired: &BTreeMap<String, Targets>,
            still_defines: &dyn Fn(&Path, &str) -> bool,
        ) -> Plan {
            plan(
                &self.state,
                &self.config,
                link_type,
                desired,
                &self.home,
                still_defines,
                None,
            )
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
            run_steps(&plan.steps, |_| {}).unwrap();
            update_records(&mut self.state, plan.records);
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

    #[test]
    fn moved_package_is_claimed_and_relinked() {
        let mut f = fixture();
        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a"), ("home/b", "repo/b")])]),
        );
        f.apply(plan);
        f.state.packages.get_mut("p").unwrap().config = f.path("old/symsync.toml");

        std::fs::create_dir_all(f.path("moved")).unwrap();
        std::fs::write(f.path("moved/a"), "moved a").unwrap();
        let plan = f.plan_with(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "moved/a")])]),
            &|_, _| false,
        );
        assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
        assert_eq!(plan.notes.len(), 1);
        assert!(messages(&plan).contains(&"unlink  ~/b"));

        f.apply(plan);
        assert_eq!(f.state.packages["p"].config, f.config);
        assert_eq!(
            std::fs::read_to_string(f.path("home/a")).unwrap(),
            "moved a"
        );
        assert!(!f.path("home/b").is_symlink());
    }

    #[test]
    fn package_removed_from_config_is_unlinked() {
        let mut f = fixture();
        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[
                ("p", &[("home/a", "repo/a")]),
                ("q", &[("home/b", "repo/b")]),
            ]),
        );
        f.apply(plan);

        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("q", &[("home/b", "repo/b")])]),
        );
        assert_eq!(messages(&plan), ["unlink  ~/a"]);
        assert_eq!(plan.records["p"], None);

        f.apply(plan);
        assert!(!f.path("home/a").is_symlink());
        assert!(!f.state.packages.contains_key("p"));
        assert!(f.path("home/b").is_symlink());
    }

    #[test]
    fn packages_of_other_configs_are_left_alone() {
        let mut f = fixture();
        f.record("q", &f.path("other/symsync.toml"), &[("home/b", "other/b")]);

        let plan = f.plan(LinkType::Absolute, &f.desired(&[]));
        assert!(plan.steps.is_empty());
        assert!(!plan.records.contains_key("q"));
    }

    #[test]
    fn remove_unlinks_package_without_config() {
        let mut f = fixture();
        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a")])]),
        );
        f.apply(plan);
        let link = f.state.link_path(&f.path("home/a"));

        let record = f.state.packages["p"].clone();
        let plan = plan_remove(&f.state, "p", &record, &f.home);
        assert_eq!(messages(&plan), ["unlink  ~/a"]);

        f.apply(plan);
        assert!(!f.path("home/a").is_symlink());
        assert!(!link.is_symlink());
        assert!(f.state.packages.is_empty());
        assert!(f.path("repo/a").is_file());
    }

    #[test]
    fn adopt_moves_drifted_file_and_relinks() {
        let mut f = fixture();
        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/a", "repo/a")])]),
        );
        f.apply(plan);
        std::fs::remove_file(f.path("home/a")).unwrap();
        std::fs::write(f.path("home/a"), "edited").unwrap();

        let record = f.state.packages["p"].clone();
        let plan = plan_adopt(&f.state, "p", &record, &f.path("home/a"), &f.home);
        assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
        assert!(plan.records.is_empty());

        f.apply(plan);
        assert_eq!(std::fs::read_to_string(f.path("repo/a")).unwrap(), "edited");
        assert!(f.path("home/a").is_symlink());
        assert_eq!(std::fs::read_to_string(f.path("home/a")).unwrap(), "edited");
    }

    #[test]
    fn scope_limits_plan_to_named_packages() {
        let mut f = fixture();
        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[
                ("p", &[("home/a", "repo/a")]),
                ("q", &[("home/b", "repo/b")]),
            ]),
        );
        f.apply(plan);
        std::fs::remove_file(f.path("home/b")).unwrap();

        // p is gone from the config and q needs a repair, but only q is in scope.
        let plan = f.plan_scoped(&f.desired(&[("q", &[("home/b", "repo/b")])]), &["q"]);
        assert_eq!(
            messages(&plan),
            [format!("link    ~/b -> {}", f.path("repo/b").display())]
        );
        assert!(!plan.records.contains_key("p"));

        let plan = f.plan_scoped(&f.desired(&[("q", &[("home/b", "repo/b")])]), &["p"]);
        assert_eq!(messages(&plan), ["unlink  ~/a"]);
        assert_eq!(plan.records["p"], None);
    }

    #[test]
    fn notes_directories_left_empty() {
        let mut f = fixture();
        let plan = f.plan(
            LinkType::Absolute,
            &f.desired(&[("p", &[("home/x/a", "repo/a"), ("home/y/b", "repo/b")])]),
        );
        f.apply(plan);
        std::fs::write(f.path("home/y/keep"), "").unwrap();

        let plan = f.plan(LinkType::Absolute, &f.desired(&[("p", &[])]));
        assert_eq!(
            plan.notes,
            ["~/x has no entries left, symsync does not remove directories"]
        );
    }
}
