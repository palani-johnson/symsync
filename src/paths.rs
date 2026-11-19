use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

/// Home directory from `$HOME`.
pub fn home_dir() -> Result<PathBuf> {
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => Ok(PathBuf::from(home)),
        _ => bail!("$HOME is not set"),
    }
}

/// Resolves a path from the config: expands a leading `~`, makes it absolute against `base`, and normalizes it.
pub fn resolve(path: &str, base: &Path, home: &Path) -> Result<PathBuf> {
    let expanded = if path == "~" {
        home.to_path_buf()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else if path.starts_with('~') {
        bail!("unsupported home path {path}, only ~ and ~/ are supported");
    } else {
        PathBuf::from(path)
    };

    Ok(normalize(&base.join(expanded)))
}

/// Removes `.` and `..` components and duplicate separators without access to the file system. Symlinks are not resolved.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !path.is_absolute() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// File name of the state link for a normalized absolute target path.
pub fn state_link_name(target: &Path) -> String {
    Sha256::digest(OsStr::as_bytes(target.as_os_str()))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/user";

    fn resolve_ok(path: &str, base: &str) -> PathBuf {
        resolve(path, Path::new(base), Path::new(HOME)).unwrap()
    }

    #[test]
    fn expands_home() {
        assert_eq!(resolve_ok("~", "/repo"), Path::new("/home/user"));
        assert_eq!(
            resolve_ok("~/.config", "/repo"),
            Path::new("/home/user/.config")
        );
    }

    #[test]
    fn rejects_other_users_home() {
        assert!(resolve("~other/x", Path::new("/repo"), Path::new(HOME)).is_err());
    }

    #[test]
    fn resolves_relative_against_base() {
        assert_eq!(resolve_ok("out", "/repo"), Path::new("/repo/out"));
        assert_eq!(resolve_ok("../out", "/repo"), Path::new("/out"));
    }

    #[test]
    fn keeps_absolute_paths() {
        assert_eq!(resolve_ok("/etc/hosts", "/repo"), Path::new("/etc/hosts"));
    }

    #[test]
    fn normalizes_lexically() {
        assert_eq!(normalize(Path::new("/a/./b//c/../d")), Path::new("/a/b/d"));
        assert_eq!(normalize(Path::new("/../a")), Path::new("/a"));
        assert_eq!(normalize(Path::new("a/../../b")), Path::new("../b"));
    }

    #[test]
    fn equal_targets_get_equal_names() {
        let a = resolve_ok("~/.zshrc", "/repo");
        let b = resolve_ok("/home/user/./x/../.zshrc", "/repo");
        assert_eq!(state_link_name(&a), state_link_name(&b));
    }

    #[test]
    fn state_link_name_is_sha256_hex() {
        assert_eq!(
            state_link_name(Path::new("abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
