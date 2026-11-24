use std::fmt;
use std::path::Path;

use crate::desired::Source;
use crate::paths::resolves_to;
use crate::state::State;

/// Health of one managed target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    MissingLink,
    Drifted,
    Exists,
    ForeignLink,
    MissingStateLink,
    WrongStateLink,
    MissingSource,
}

impl fmt::Display for Health {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ok => "ok",
            Self::MissingLink => "missing link",
            Self::Drifted => "drifted",
            Self::Exists => "exists",
            Self::ForeignLink => "foreign link",
            Self::MissingStateLink => "missing state link",
            Self::WrongStateLink => "wrong state link",
            Self::MissingSource => "missing source",
        })
    }
}

/// Checks the link chain from `target` to `source`.
pub fn check(state: &State, target: &Path, source: &Source) -> Health {
    let link = state.link_path(target);
    let expected = if source.direct { &source.path } else { &link };

    match std::fs::symlink_metadata(target) {
        Err(_) => return Health::MissingLink,
        Ok(meta) if meta.is_file() => return Health::Drifted,
        Ok(meta) if !meta.file_type().is_symlink() => return Health::Exists,
        Ok(_) => {}
    }
    match std::fs::read_link(target) {
        Ok(value) if resolves_to(&value, target, expected) => {}
        _ => return Health::ForeignLink,
    }

    if !source.direct {
        match std::fs::read_link(&link) {
            Err(_) => return Health::MissingStateLink,
            Ok(value) if !resolves_to(&value, &link, &source.path) => {
                return Health::WrongStateLink;
            }
            Ok(_) => {}
        }
    }

    if source.path.is_file() {
        Health::Ok
    } else {
        Health::MissingSource
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        state: State,
        target: PathBuf,
        source: PathBuf,
        link: PathBuf,
    }

    fn linked() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let state = State::open(&tmp.path().join("state")).unwrap();
        let target = tmp.path().join("target");
        let source = tmp.path().join("source");
        let link = state.link_path(&target);
        std::fs::write(&source, "").unwrap();
        symlink(&source, &link).unwrap();
        symlink(&link, &target).unwrap();
        Fixture {
            _tmp: tmp,
            state,
            target,
            source,
            link,
        }
    }

    impl Fixture {
        fn check(&self) -> Health {
            self.check_as(false)
        }

        fn check_as(&self, direct: bool) -> Health {
            let source = Source {
                path: self.source.clone(),
                direct,
            };
            check(&self.state, &self.target, &source)
        }
    }

    #[test]
    fn full_chain_is_ok() {
        assert_eq!(linked().check(), Health::Ok);
    }

    #[test]
    fn detects_missing_link() {
        let f = linked();
        std::fs::remove_file(&f.target).unwrap();
        assert_eq!(f.check(), Health::MissingLink);
    }

    #[test]
    fn detects_drift() {
        let f = linked();
        std::fs::remove_file(&f.target).unwrap();
        std::fs::write(&f.target, "").unwrap();
        assert_eq!(f.check(), Health::Drifted);
    }

    #[test]
    fn detects_foreign_link() {
        let f = linked();
        std::fs::remove_file(&f.target).unwrap();
        symlink(&f.source, &f.target).unwrap();
        assert_eq!(f.check(), Health::ForeignLink);
    }

    #[test]
    fn detects_state_link_problems() {
        let f = linked();
        std::fs::remove_file(&f.link).unwrap();
        assert_eq!(f.check(), Health::MissingStateLink);

        symlink("/elsewhere", &f.link).unwrap();
        assert_eq!(f.check(), Health::WrongStateLink);
    }

    #[test]
    fn detects_missing_source() {
        let f = linked();
        std::fs::remove_file(&f.source).unwrap();
        assert_eq!(f.check(), Health::MissingSource);
    }

    #[test]
    fn direct_link_skips_state_link() {
        let f = linked();
        assert_eq!(f.check_as(true), Health::ForeignLink);

        std::fs::remove_file(&f.target).unwrap();
        std::fs::remove_file(&f.link).unwrap();
        symlink(&f.source, &f.target).unwrap();
        assert_eq!(f.check_as(true), Health::Ok);
        assert_eq!(f.check(), Health::ForeignLink);
    }

    #[test]
    fn directory_at_target_is_not_drift() {
        let f = linked();
        std::fs::remove_file(&f.target).unwrap();
        std::fs::create_dir(&f.target).unwrap();
        assert_eq!(f.check(), Health::Exists);
    }
}
