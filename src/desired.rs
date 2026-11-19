use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::LoadedConfig;
use crate::paths::{normalize, resolve};

/// Source of each target, keyed by target. All paths are absolute.
pub type Targets = BTreeMap<PathBuf, PathBuf>;

/// Expands the globs of every package in the config into the targets the package wants. The config file and files that belong to the state directory at `state_dir` are never sources.
pub fn expand(
    loaded: &LoadedConfig,
    home: &Path,
    state_dir: &Path,
) -> Result<BTreeMap<String, Targets>> {
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
                for path in matches(&root, source, group.hidden)? {
                    if path == loaded.path || belongs_to_state(&path, state_dir) {
                        continue;
                    }
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

/// Whether `path` is in the state directory, or is a link into it. Without this check, a target directory inside a root feeds its own links back in as sources.
fn belongs_to_state(path: &Path, state_dir: &Path) -> bool {
    if path.starts_with(state_dir) {
        return true;
    }
    std::fs::read_link(path).is_ok_and(|value| {
        let base = path.parent().unwrap_or(Path::new("/"));
        normalize(&base.join(value)).starts_with(state_dir)
    })
}

/// Files that match `pattern` under `root`, normalized. Unless `hidden` is set, a name that starts with `.` matches only a literal `.` in the pattern.
fn matches(root: &Path, pattern: &str, hidden: bool) -> Result<Vec<PathBuf>> {
    let invalid = || format!("invalid glob {pattern}");
    let full = format!(
        "{}/{pattern}",
        glob::Pattern::escape(&root.to_string_lossy())
    );
    let paths = glob::glob(&full).with_context(invalid)?;

    // The glob walker drops every hidden entry when `require_literal_leading_dot` is set, even for `.*`, so the rule is applied to each match instead.
    let relative = glob::Pattern::new(pattern).with_context(invalid)?;
    let options = glob::MatchOptions {
        require_literal_leading_dot: !hidden,
        ..glob::MatchOptions::new()
    };

    let mut files = Vec::new();
    for path in paths {
        let path = path.with_context(|| format!("failed to read a match of {pattern}"))?;
        let visible = path
            .strip_prefix(root)
            .is_ok_and(|rel| relative.matches_path_with(rel, options));
        if visible && path.is_file() {
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
        state: PathBuf,
    }

    impl Fixture {
        fn expand(&self, loaded: &LoadedConfig) -> Result<BTreeMap<String, Targets>> {
            expand(loaded, &self.home, &self.state)
        }
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
            state: repo.join(".symsync.state"),
            repo,
            home,
        }
    }

    fn group(root: Option<&str>, sources: &[&str], target: &str) -> LinkGroup {
        LinkGroup {
            root: root.map(PathBuf::from),
            sources: sources.iter().map(|s| s.to_string()).collect(),
            target: target.into(),
            hidden: false,
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

        let targets = &f.expand(&loaded).unwrap()["zed"];
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
            f.expand(&loaded).unwrap()["p"],
            Targets::from([(f.repo.join("out/a.txt"), f.repo.join("a.txt"))])
        );
    }

    #[test]
    fn skips_dirs() {
        let f = fixture(&["zsh/zshrc", "zsh/dir/file"]);
        let loaded = config(&f, vec![("zsh", vec![group(Some("zsh"), &["*"], "~")])]);

        assert_eq!(
            f.expand(&loaded).unwrap()["zsh"],
            Targets::from([(f.home.join("zshrc"), f.repo.join("zsh/zshrc"))])
        );
    }

    #[test]
    fn wildcards_skip_hidden_files_by_default() {
        let f = fixture(&["a", ".hidden", ".git/config", "dir/.hidden"]);
        let loaded = config(&f, vec![("p", vec![group(None, &["**/*"], "out")])]);

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([(f.repo.join("out/a"), f.repo.join("a"))])
        );
    }

    #[test]
    fn literal_dot_matches_hidden_files() {
        let f = fixture(&["zsh/.zshrc", "zsh/.zshenv", "zsh/README"]);
        let loaded = config(&f, vec![("zsh", vec![group(Some("zsh"), &[".zsh*"], "~")])]);

        assert_eq!(f.expand(&loaded).unwrap()["zsh"].len(), 2);
    }

    #[test]
    fn hidden_lets_wildcards_match_hidden_files() {
        let f = fixture(&["a", ".hidden", "dir/.hidden"]);
        let mut hidden = group(None, &["**/*"], "out");
        hidden.hidden = true;
        let loaded = config(&f, vec![("p", vec![hidden])]);

        assert_eq!(f.expand(&loaded).unwrap()["p"].len(), 3);
    }

    #[test]
    fn same_target_from_same_source_is_allowed() {
        let f = fixture(&["a.txt"]);
        let loaded = config(&f, vec![("p", vec![group(None, &["*.txt", "a.*"], "out")])]);
        assert_eq!(f.expand(&loaded).unwrap()["p"].len(), 1);
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
        assert!(f.expand(&loaded).is_err());
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
        assert!(f.expand(&loaded).is_err());
    }

    #[test]
    fn skips_state_dir_and_managed_links() {
        let f = fixture(&["a.txt", ".symsync.state/links/x"]);
        std::fs::create_dir_all(f.repo.join("out")).unwrap();
        std::os::unix::fs::symlink("../.symsync.state/links/x", f.repo.join("out/a.txt")).unwrap();

        let mut all = group(None, &["**/*"], "out");
        all.hidden = true;
        let loaded = config(&f, vec![("p", vec![all])]);

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([(f.repo.join("out/a.txt"), f.repo.join("a.txt"))])
        );
    }

    #[test]
    fn skips_config_file() {
        let f = fixture(&["symsync.toml", "a.txt"]);
        let loaded = config(&f, vec![("p", vec![group(None, &["*"], "out")])]);

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([(f.repo.join("out/a.txt"), f.repo.join("a.txt"))])
        );
    }
}
