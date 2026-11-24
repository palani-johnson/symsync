use std::collections::BTreeMap;
use std::os::unix::fs::symlink;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::plan::{Action, Step};
use crate::state::{PackageRecord, State};

/// Runs the steps of a plan and saves its package records.
///
/// Before any change, each changed package is saved with the union of its old and new targets, and a target in both keeps its old source. If a step fails, every link that can be on disk is still recorded, and the next sync cleans up.
pub fn apply(
    state: &mut State,
    steps: &[Step],
    records: BTreeMap<String, Option<PackageRecord>>,
    report: impl FnMut(&str),
) -> Result<()> {
    if !steps.is_empty() {
        for (name, record) in &records {
            let Some(new) = record else { continue };
            let mut merged = new.clone();
            if let Some(old) = state.packages.get(name) {
                for (target, source) in &old.targets {
                    merged.targets.insert(target.clone(), source.clone());
                }
            }
            state.packages.insert(name.clone(), merged);
        }
        state.save()?;
        run_steps(steps, report)?;
    }

    update_records(state, records);
    state.save()
}

/// Runs steps in order and stops at the first error. `report` gets the message of each step before it runs.
pub fn run_steps(steps: &[Step], mut report: impl FnMut(&str)) -> Result<()> {
    for step in steps {
        if let Some(message) = &step.message {
            report(message);
        }
        run(&step.action)?;
    }
    Ok(())
}

fn run(action: &Action) -> Result<()> {
    match action {
        Action::CreateDir(dir) => std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display())),
        Action::CreateLink { path, value } => create_link(path, value),
        Action::ReplaceLink { path, value } => {
            remove_link(path)?;
            create_link(path, value)
        }
        Action::RemoveLink(path) => remove_link(path),
        Action::MoveFile { from, to } => move_file(from, to),
    }
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(from)
        .with_context(|| format!("failed to read {}", from.display()))?;
    if !meta.is_file() {
        bail!("{} is not a regular file", from.display());
    }
    std::fs::copy(from, to)
        .with_context(|| format!("failed to copy {} to {}", from.display(), to.display()))?;
    std::fs::remove_file(from).with_context(|| format!("failed to remove {}", from.display()))
}

fn create_link(path: &Path, value: &Path) -> Result<()> {
    symlink(value, path).with_context(|| format!("failed to create link {}", path.display()))
}

fn remove_link(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if !meta.file_type().is_symlink() {
        bail!(
            "{} is no longer a symlink, it was not removed",
            path.display()
        );
    }
    std::fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
}

/// Puts the package records of a plan into the state. Does not save the state.
pub fn update_records(state: &mut State, records: BTreeMap<String, Option<PackageRecord>>) {
    for (name, record) in records {
        match record {
            Some(record) => state.packages.insert(name, record),
            None => state.packages.remove(&name),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(action: Action) -> Step {
        Step {
            message: None,
            action,
        }
    }

    #[test]
    fn creates_replaces_and_removes_links() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("dir/link");

        run_steps(
            &[
                step(Action::CreateDir(tmp.path().join("dir"))),
                step(Action::CreateLink {
                    path: link.clone(),
                    value: "a".into(),
                }),
            ],
            |_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("a"));

        run_steps(
            &[step(Action::ReplaceLink {
                path: link.clone(),
                value: "b".into(),
            })],
            |_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("b"));

        run_steps(&[step(Action::RemoveLink(link.clone()))], |_| {}).unwrap();
        assert!(!link.is_symlink());
    }

    #[test]
    fn does_not_remove_regular_files() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        std::fs::write(&file, "").unwrap();

        assert!(run_steps(&[step(Action::RemoveLink(file.clone()))], |_| {}).is_err());
        assert!(file.is_file());
    }

    #[test]
    fn does_not_overwrite_on_create() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        std::fs::write(&file, "").unwrap();

        let create = step(Action::CreateLink {
            path: file.clone(),
            value: "a".into(),
        });
        assert!(run_steps(&[create], |_| {}).is_err());
        assert!(file.is_file());
    }

    #[test]
    fn reports_messages_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let steps = [
            Step {
                message: Some("one".into()),
                action: Action::CreateDir(tmp.path().join("a")),
            },
            step(Action::CreateDir(tmp.path().join("b"))),
            Step {
                message: Some("two".into()),
                action: Action::CreateDir(tmp.path().join("c")),
            },
        ];

        let mut seen = Vec::new();
        run_steps(&steps, |m| seen.push(m.to_string())).unwrap();
        assert_eq!(seen, ["one", "two"]);
    }

    #[test]
    fn moves_file_content_over_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from");
        let to = tmp.path().join("to");
        std::fs::write(&from, "new").unwrap();
        std::fs::write(&to, "old").unwrap();

        let action = Action::MoveFile {
            from: from.clone(),
            to: to.clone(),
        };
        run_steps(&[step(action)], |_| {}).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "new");
        assert!(!from.exists());
    }

    #[test]
    fn does_not_move_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from");
        let to = tmp.path().join("to");
        std::fs::write(&to, "old").unwrap();
        symlink(&to, &from).unwrap();

        let action = Action::MoveFile {
            from: from.clone(),
            to: to.clone(),
        };
        assert!(run_steps(&[step(action)], |_| {}).is_err());
        assert!(from.is_symlink());
    }

    #[test]
    fn failed_apply_keeps_old_and_new_targets_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state");
        let record = |targets: &[&str]| PackageRecord {
            config: tmp.path().join("symsync.toml"),
            link_type: crate::config::LinkType::Absolute,
            targets: targets
                .iter()
                .map(|t| {
                    let source = crate::desired::Source {
                        path: tmp.path().join("source"),
                        direct: false,
                    };
                    (tmp.path().join(t), source)
                })
                .collect(),
        };

        let mut state = State::open(&dir).unwrap();
        state.packages.insert("p".into(), record(&["old"]));
        let records = BTreeMap::from([("p".to_string(), Some(record(&["new"])))]);
        let failing = step(Action::RemoveLink(tmp.path().join("missing")));

        assert!(apply(&mut state, &[failing], records, |_| {}).is_err());
        drop(state);

        let saved = State::read(&dir).unwrap();
        let targets: Vec<_> = saved.packages["p"].targets.keys().collect();
        assert_eq!(targets, [&tmp.path().join("new"), &tmp.path().join("old")]);
    }

    #[test]
    fn successful_apply_saves_new_records() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state");
        let mut state = State::open(&dir).unwrap();
        state.packages.insert(
            "p".into(),
            PackageRecord {
                config: tmp.path().join("symsync.toml"),
                link_type: crate::config::LinkType::Absolute,
                targets: Default::default(),
            },
        );

        let create = step(Action::CreateDir(tmp.path().join("d")));
        let records = BTreeMap::from([("p".to_string(), None)]);
        apply(&mut state, &[create], records, |_| {}).unwrap();
        drop(state);

        assert!(State::read(&dir).unwrap().packages.is_empty());
    }
}
