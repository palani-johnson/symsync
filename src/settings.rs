use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::config::{LinkType, LoadedConfig};
use crate::paths::{self, normalize};

/// Process inputs that settings are resolved from.
#[derive(Debug, Clone)]
pub struct Env {
    pub cwd: PathBuf,
    pub home: PathBuf,
    pub state_dir: Option<OsString>,
    pub link_type: Option<OsString>,
    pub xdg_state_home: Option<OsString>,
}

impl Env {
    pub fn from_process() -> Result<Self> {
        let var = |name| std::env::var_os(name).filter(|value| !value.is_empty());

        Ok(Self {
            cwd: std::env::current_dir()?,
            home: paths::home_dir()?,
            state_dir: var("SYMSYNC_STATE_DIR"),
            link_type: var("SYMSYNC_LINK_TYPE"),
            xdg_state_home: var("XDG_STATE_HOME"),
        })
    }
}

/// Resolves the state directory from, in order: the flag, `$SYMSYNC_STATE_DIR`, the config, `$XDG_STATE_HOME/symsync`, and `~/.local/state/symsync`.
pub fn state_dir(flag: Option<&Path>, env: &Env, config: Option<&LoadedConfig>) -> PathBuf {
    if let Some(path) = flag {
        return normalize(&env.cwd.join(path));
    }
    if let Some(path) = &env.state_dir {
        return normalize(&env.cwd.join(path));
    }
    if let Some(loaded) = config
        && let Some(path) = &loaded.config.state_dir
    {
        return normalize(&loaded.dir().join(path));
    }
    // The XDG spec says to ignore a relative value.
    if let Some(xdg) = &env.xdg_state_home
        && Path::new(xdg).is_absolute()
    {
        return normalize(&Path::new(xdg).join("symsync"));
    }
    env.home.join(".local/state/symsync")
}

/// Resolves the link type from, in order: `$SYMSYNC_LINK_TYPE`, the config, and the default.
pub fn link_type(env: &Env, config: Option<&LoadedConfig>) -> Result<LinkType> {
    if let Some(value) = &env.link_type {
        return match value.to_str() {
            Some("absolute") => Ok(LinkType::Absolute),
            Some("relative") => Ok(LinkType::Relative),
            _ => bail!(
                "invalid $SYMSYNC_LINK_TYPE {}, expected absolute or relative",
                value.display()
            ),
        };
    }
    Ok(config
        .and_then(|loaded| loaded.config.link_type)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::Config;

    fn env() -> Env {
        Env {
            cwd: PathBuf::from("/cwd"),
            home: PathBuf::from("/home/user"),
            state_dir: None,
            link_type: None,
            xdg_state_home: None,
        }
    }

    fn loaded(state_dir: Option<&str>, link_type: Option<LinkType>) -> LoadedConfig {
        LoadedConfig {
            path: PathBuf::from("/repo/symsync.toml"),
            config: Config {
                state_dir: state_dir.map(PathBuf::from),
                link_type,
                git_tracked: None,
                direct: None,
                packages: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn state_dir_precedence() {
        let mut env = env();
        let config = loaded(Some(".state"), None);

        assert_eq!(
            state_dir(None, &env, None),
            Path::new("/home/user/.local/state/symsync")
        );

        env.xdg_state_home = Some("/xdg".into());
        assert_eq!(state_dir(None, &env, None), Path::new("/xdg/symsync"));

        assert_eq!(
            state_dir(None, &env, Some(&config)),
            Path::new("/repo/.state")
        );

        env.state_dir = Some("env-state".into());
        assert_eq!(
            state_dir(None, &env, Some(&config)),
            Path::new("/cwd/env-state")
        );

        assert_eq!(
            state_dir(Some(Path::new("/flag")), &env, Some(&config)),
            Path::new("/flag")
        );
    }

    #[test]
    fn relative_xdg_state_home_is_ignored() {
        let mut env = env();
        env.xdg_state_home = Some("relative".into());
        assert_eq!(
            state_dir(None, &env, None),
            Path::new("/home/user/.local/state/symsync")
        );
    }

    #[test]
    fn link_type_precedence() {
        let mut env = env();
        let config = loaded(None, Some(LinkType::Relative));

        assert_eq!(link_type(&env, None).unwrap(), LinkType::Absolute);
        assert_eq!(link_type(&env, Some(&config)).unwrap(), LinkType::Relative);

        env.link_type = Some("absolute".into());
        assert_eq!(link_type(&env, Some(&config)).unwrap(), LinkType::Absolute);
    }

    #[test]
    fn rejects_invalid_link_type() {
        let mut env = env();
        env.link_type = Some("sideways".into());
        assert!(link_type(&env, None).is_err());
    }
}
