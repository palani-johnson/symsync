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

/// Resolves symlinks in the longest part of a normalized absolute path that exists. The rest of the path is kept as it is.
pub fn canonicalize_existing(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(mut real) = std::fs::canonicalize(current) {
            real.extend(missing.iter().rev());
            return real;
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_owned());
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// File name of the state link for a normalized absolute target path.
pub fn state_link_name(target: &Path) -> String {
    Sha256::digest(OsStr::as_bytes(target.as_os_str()))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether the symlink at `at`, with the content `value`, points to `expected`.
pub fn resolves_to(value: &Path, at: &Path, expected: &Path) -> bool {
    let base = at.parent().unwrap_or(Path::new("/"));
    normalize(&base.join(value)) == expected
}

/// `path` relative to the directory `from`.
pub fn relative(path: &Path, from: &Path) -> PathBuf {
    pathdiff::diff_paths(path, from).unwrap_or_else(|| path.to_path_buf())
}

/// `path` for output, with `~` for the home directory.
pub fn display(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
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

    #[test]
    fn resolves_absolute_and_relative_values() {
        let expected = Path::new("/state/links/x");
        assert!(resolves_to(expected, Path::new("/home/a"), expected));
        assert!(resolves_to(
            Path::new("../state/links/x"),
            Path::new("/home/a"),
            expected
        ));
        assert!(!resolves_to(
            Path::new("state/links/x"),
            Path::new("/home/a"),
            expected
        ));
    }

    #[test]
    fn displays_home_as_tilde() {
        let home = Path::new(HOME);
        assert_eq!(display(Path::new("/home/user/.zshrc"), home), "~/.zshrc");
        assert_eq!(display(Path::new("/home/user"), home), "~");
        assert_eq!(display(Path::new("/etc/hosts"), home), "/etc/hosts");
    }

    #[test]
    fn canonicalizes_existing_part_only() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().canonicalize().unwrap();
        std::fs::create_dir(real.join("dir")).unwrap();
        std::os::unix::fs::symlink("dir", real.join("alias")).unwrap();

        assert_eq!(
            canonicalize_existing(&real.join("alias/new/state")),
            real.join("dir/new/state")
        );
    }
}
