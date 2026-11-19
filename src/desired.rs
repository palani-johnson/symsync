use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::LoadedConfig;
use crate::paths::{normalize, resolve};

/// Source of each target, keyed by target. All paths are absolute.
pub type Targets = BTreeMap<PathBuf, PathBuf>;

/// Expands the globs of every package in the config into the targets the package wants.
pub fn expand(loaded: &LoadedConfig, home: &Path) -> Result<BTreeMap<String, Targets>> {
    let mut packages = BTreeMap::new();
    let mut owners: BTreeMap<PathBuf, (String, PathBuf)> = BTreeMap::new();

    for (name, package) in &loaded.config.packages {
        let mut targets = Targets::new();

        for group in &package.links {
            let root = match &group.root {
                Some(root) => resolve(&root.to_string_lossy(), loaded.dir(), home)?,
                None => loaded.dir().to_path_buf(),
            };
            let target_dir = resolve(&group.target, loaded.dir(), home)?;

            for source in &group.sources {
                for path in matches(&root, source)? {
                    let rel = path.strip_prefix(&root).with_context(|| {
                        format!("{} is outside of root {}", path.display(), root.display())
                    })?;
                    let target = normalize(&target_dir.join(rel));

                    if let Some((owner, owner_source)) = owners.get(&target)
                        && (owner != name || owner_source != &path)
                    {
                        bail!(
                            "target {} is wanted by {} (package {owner}) and {} (package {name})",
                            target.display(),
                            owner_source.display(),
                            path.display()
                        );
                    }
                    owners.insert(target.clone(), (name.clone(), path.clone()));
                    targets.insert(target, path);
                }
            }
        }

        packages.insert(name.clone(), targets);
    }

    Ok(packages)
}

/// Files that match `pattern` under `root`, normalized.
fn matches(root: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let full = format!(
        "{}/{pattern}",
        glob::Pattern::escape(&root.to_string_lossy())
    );
    let paths = glob::glob(&full).with_context(|| format!("invalid glob {pattern}"))?;

    let mut files = Vec::new();
    for path in paths {
        let path = path.with_context(|| format!("failed to read a match of {pattern}"))?;
        if path.is_file() {
            files.push(normalize(&path));
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, LinkGroup, Package};

    struct Fixture {
        _tmp: tempfile::TempDir,
        repo: PathBuf,
        home: PathBuf,
    }

    fn fixture(files: &[&str]) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        for file in files {
            let path = repo.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, file).unwrap();
        }
        Fixture {
            _tmp: tmp,
            repo,
            home,
        }
    }

    fn group(root: Option<&str>, sources: &[&str], target: &str) -> LinkGroup {
        LinkGroup {
            root: root.map(PathBuf::from),
            sources: sources.iter().map(|s| s.to_string()).collect(),
            target: target.into(),
        }
    }

    fn config(fixture: &Fixture, packages: Vec<(&str, Vec<LinkGroup>)>) -> LoadedConfig {
        LoadedConfig {
            path: fixture.repo.join("symsync.toml"),
            config: Config {
                state_dir: None,
                link_type: None,
                packages: packages
                    .into_iter()
                    .map(|(name, links)| (name.into(), Package { links }))
                    .collect(),
            },
        }
    }

    #[test]
    fn maps_matches_relative_to_root() {
        let f = fixture(&["zed/settings.json", "zed/themes/dark.json", "zed/notes.md"]);
        let loaded = config(
            &f,
            vec![(
                "zed",
                vec![group(Some("zed"), &["**/*.json"], "~/.config/zed")],
            )],
        );

        let targets = &expand(&loaded, &f.home).unwrap()["zed"];
        assert_eq!(
            targets,
            &Targets::from([
                (
                    f.home.join(".config/zed/settings.json"),
                    f.repo.join("zed/settings.json")
                ),
                (
                    f.home.join(".config/zed/themes/dark.json"),
                    f.repo.join("zed/themes/dark.json")
                ),
            ])
        );
    }

    #[test]
    fn root_defaults_to_config_dir() {
        let f = fixture(&["a.txt"]);
        let loaded = config(&f, vec![("p", vec![group(None, &["*.txt"], "out")])]);

        assert_eq!(
            expand(&loaded, &f.home).unwrap()["p"],
            Targets::from([(f.repo.join("out/a.txt"), f.repo.join("a.txt"))])
        );
    }

    #[test]
    fn matches_dotfiles_and_skips_dirs() {
        let f = fixture(&["zsh/.zshrc", "zsh/dir/file"]);
        let loaded = config(&f, vec![("zsh", vec![group(Some("zsh"), &["*"], "~")])]);

        assert_eq!(
            expand(&loaded, &f.home).unwrap()["zsh"],
            Targets::from([(f.home.join(".zshrc"), f.repo.join("zsh/.zshrc"))])
        );
    }

    #[test]
    fn same_target_from_same_source_is_allowed() {
        let f = fixture(&["a.txt"]);
        let loaded = config(&f, vec![("p", vec![group(None, &["*.txt", "a.*"], "out")])]);
        assert_eq!(expand(&loaded, &f.home).unwrap()["p"].len(), 1);
    }

    #[test]
    fn rejects_target_wanted_by_two_sources() {
        let f = fixture(&["a/x.txt", "b/x.txt"]);
        let loaded = config(
            &f,
            vec![(
                "p",
                vec![
                    group(Some("a"), &["*"], "out"),
                    group(Some("b"), &["*"], "out"),
                ],
            )],
        );
        assert!(expand(&loaded, &f.home).is_err());
    }

    #[test]
    fn rejects_target_wanted_by_two_packages() {
        let f = fixture(&["a.txt"]);
        let loaded = config(
            &f,
            vec![
                ("p", vec![group(None, &["*.txt"], "out")]),
                ("q", vec![group(None, &["*.txt"], "out")]),
            ],
        );
        assert!(expand(&loaded, &f.home).is_err());
    }
}
