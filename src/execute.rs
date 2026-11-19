use std::collections::BTreeMap;
use std::os::unix::fs::symlink;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::plan::{Action, Step};
use crate::state::{PackageRecord, State};

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
}
