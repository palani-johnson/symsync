use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use wax::walk::Entry;
use wax::{CandidatePath, Glob, MatchedText, Program};

use crate::config::{LoadedConfig, Rename};
use crate::paths::{display, normalize, resolve};

/// Source of each target, keyed by target. All paths are absolute.
pub type Targets = BTreeMap<PathBuf, PathBuf>;

/// The targets that each package wants.
#[derive(Debug, Default)]
pub struct Expansion {
    pub packages: BTreeMap<String, Targets>,
    /// Problems that do not stop sync, such as a glob that matches no files, with the package they belong to.
    pub warnings: Vec<(String, String)>,
}

/// Checks that the state directory and all paths of all packages are inside `dir`. A relative chain survives a move only when everything moves together.
pub fn check_contained(
    packages: &BTreeMap<String, Targets>,
    state_dir: &Path,
    dir: &Path,
    home: &Path,
) -> Result<()> {
    let outside = std::iter::once(state_dir)
        .chain(
            packages
                .values()
                .flat_map(|targets| targets.iter())
                .flat_map(|(target, source)| [target.as_path(), source.as_path()]),
        )
        .find(|path| !path.starts_with(dir));

    if let Some(path) = outside {
        bail!(
            "the relative link type needs the state directory and all targets inside {}, but {} is outside",
            display(dir, home),
            display(path, home)
        );
    }
    Ok(())
}

/// Expands the globs of every package in the config into the targets the package wants. The config file and files that belong to the state directory at `state_dir` are never sources.
pub fn expand(loaded: &LoadedConfig, home: &Path, state_dir: &Path) -> Result<Expansion> {
    let mut expansion = Expansion::default();
    let mut owners: BTreeMap<PathBuf, (String, PathBuf)> = BTreeMap::new();

    for (name, package) in &loaded.config.packages {
        let mut targets = Targets::new();

        for group in &package.links {
            let root = match &group.root {
                Some(root) => resolve(&root.to_string_lossy(), loaded.dir(), home)?,
                None => normalize(&loaded.dir().join(name)),
            };
            let target_dir = resolve(&group.target, loaded.dir(), home)?;
            let rules = group
                .rename
                .iter()
                .map(RenameRule::new)
                .collect::<Result<Vec<_>>>()
                .with_context(|| format!("invalid rename in package {name}"))?;
            let mut used = vec![false; rules.len()];
            let git_tracked = group
                .git_tracked
                .or(package.git_tracked)
                .or(loaded.config.git_tracked)
                .unwrap_or(true);
            let tracked = if git_tracked {
                Some(tracked_files(&root).with_context(|| {
                    format!("failed to list git-tracked files for package {name}")
                })?)
            } else {
                None
            };

            for source in &group.sources {
                let mut matched = false;
                for path in matches(&root, source)? {
                    if path == loaded.path || belongs_to_state(&path, state_dir) {
                        continue;
                    }
                    if tracked.as_ref().is_some_and(|files| !files.contains(&path)) {
                        continue;
                    }
                    matched = true;
                    let rel = path.strip_prefix(&root).with_context(|| {
                        format!("{} is outside of root {}", path.display(), root.display())
                    })?;
                    let rel = rename(&rules, rel, &mut used)
                        .with_context(|| format!("failed to rename in package {name}"))?;
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
                if !matched {
                    expansion.warnings.push((
                        name.clone(),
                        format!("glob {source} in {} matched no files", display(&root, home)),
                    ));
                }
            }

            for (rule, used) in rules.iter().zip(used) {
                if !used {
                    expansion.warnings.push((
                        name.clone(),
                        format!(
                            "rename {} in {} renamed no files",
                            rule.from_text,
                            display(&root, home)
                        ),
                    ));
                }
            }
        }

        expansion.packages.insert(name.clone(), targets);
    }

    Ok(expansion)
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

/// Files under `root` that are in the git index, normalized.
fn tracked_files(root: &Path) -> Result<BTreeSet<PathBuf>> {
    if !root.is_dir() {
        return Ok(BTreeSet::new());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .context("failed to run git")?;
    if !output.status.success() {
        bail!(
            "git ls-files in {} failed: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(output
        .stdout
        .split(|&b| b == 0)
        .filter(|rel| !rel.is_empty())
        .map(|rel| normalize(&root.join(OsStr::from_bytes(rel))))
        .collect())
}

/// Files that match `pattern` under `root`, normalized.
fn matches(root: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let glob = Glob::new(pattern).with_context(|| format!("invalid glob {pattern}"))?;
    if pattern.starts_with('/') || glob.has_semantic_literals() {
        bail!("glob {pattern} must be relative to root, without . or .. components");
    }
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in glob.walk(root) {
        let entry = entry.with_context(|| format!("failed to read a match of {pattern}"))?;
        let path = entry.path();
        if path.is_file() {
            files.push(normalize(path));
        }
    }
    Ok(files)
}

struct RenameRule {
    from: Glob<'static>,
    from_text: String,
    to: Vec<Part>,
}

enum Part {
    Text(String),
    Capture(usize),
}

impl RenameRule {
    fn new(rule: &Rename) -> Result<Self> {
        let from = Glob::new(&rule.from)
            .with_context(|| format!("invalid glob {}", rule.from))?
            .into_owned();
        let to = parse_template(&rule.to)
            .with_context(|| format!("invalid rename target {}", rule.to))?;

        let captures = from.captures().count();
        if let Some(index) = to.iter().find_map(|part| match part {
            Part::Capture(index) if *index > captures => Some(*index),
            _ => None,
        }) {
            bail!(
                "{} refers to capture {index}, but {} has {captures}",
                rule.to,
                rule.from
            );
        }

        Ok(Self {
            from,
            from_text: rule.from.clone(),
            to,
        })
    }

    fn fill(&self, matched: &MatchedText) -> String {
        self.to
            .iter()
            .map(|part| match part {
                Part::Text(text) => text.as_str(),
                Part::Capture(index) => matched.get(*index).unwrap_or_default(),
            })
            .collect()
    }
}

fn parse_template(template: &str) -> Result<Vec<Part>> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut index = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => index.push(c),
                        None => bail!("unclosed {{, use {{{{ for a literal brace"),
                    }
                }
                let index = index
                    .parse()
                    .with_context(|| format!("{{{index}}} is not a capture number"))?;
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(Part::Capture(index));
            }
            '}' => bail!("unmatched }}, use }}}} for a literal brace"),
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    Ok(parts)
}

/// The path of `rel` at the target: the result of the first rule that matches it, or `rel` itself. Marks the rule that applies in `used`.
fn rename(rules: &[RenameRule], rel: &Path, used: &mut [bool]) -> Result<PathBuf> {
    let text = rel
        .to_str()
        .with_context(|| format!("{} is not a UTF-8 path", rel.display()))?;
    let candidate = CandidatePath::from(text);

    let Some((index, rule, matched)) = rules
        .iter()
        .enumerate()
        .find_map(|(i, rule)| Some((i, rule, rule.from.matched(&candidate)?)))
    else {
        return Ok(rel.to_path_buf());
    };
    used[index] = true;

    let renamed = PathBuf::from(rule.fill(&matched));
    let valid = renamed.components().next().is_some()
        && renamed
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !valid {
        bail!(
            "rename {} turns {text} into {}, which is not a relative path inside the target",
            rule.from_text,
            renamed.display()
        );
    }
    Ok(renamed)
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
            expand(loaded, &self.home, &self.state).map(|e| e.packages)
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
            git_tracked: None,
            rename: Vec::new(),
        }
    }

    fn git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn renamed(sources: &[&str], rules: &[(&str, &str)]) -> LinkGroup {
        let mut group = group(Some("."), sources, "out");
        group.rename = rules
            .iter()
            .map(|(from, to)| Rename {
                from: from.to_string(),
                to: to.to_string(),
            })
            .collect();
        group
    }

    /// A config with `git_tracked` off, because most fixtures are not git repos.
    fn config(fixture: &Fixture, packages: Vec<(&str, Vec<LinkGroup>)>) -> LoadedConfig {
        LoadedConfig {
            path: fixture.repo.join("symsync.toml"),
            config: Config {
                state_dir: None,
                link_type: None,
                git_tracked: Some(false),
                packages: packages
                    .into_iter()
                    .map(|(name, links)| {
                        let package = Package {
                            git_tracked: None,
                            links,
                        };
                        (name.into(), package)
                    })
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
    fn root_defaults_to_package_name() {
        let f = fixture(&["p/a.txt", "b.txt"]);
        let loaded = config(&f, vec![("p", vec![group(None, &["*.txt"], "out")])]);

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([(f.repo.join("out/a.txt"), f.repo.join("p/a.txt"))])
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
    fn wildcards_match_hidden_files() {
        let f = fixture(&["a", ".hidden", "dir/.hidden"]);
        let loaded = config(&f, vec![("p", vec![group(Some("."), &["**/*"], "out")])]);

        assert_eq!(f.expand(&loaded).unwrap()["p"].len(), 3);
    }

    #[test]
    fn alternations_select_files() {
        let f = fixture(&[".zshrc", ".zshenv", ".other"]);
        let loaded = config(
            &f,
            vec![("p", vec![group(Some("."), &["{.zshrc,.zshenv}"], "out")])],
        );

        assert_eq!(f.expand(&loaded).unwrap()["p"].len(), 2);
    }

    #[test]
    fn rejects_globs_outside_root() {
        let f = fixture(&["a.txt"]);
        for pattern in ["../a.txt", "/a.txt", "./a.txt"] {
            let loaded = config(&f, vec![("p", vec![group(Some("."), &[pattern], "out")])]);
            assert!(f.expand(&loaded).is_err(), "{pattern}");
        }
    }

    #[test]
    fn missing_root_only_warns() {
        let f = fixture(&["a.txt"]);
        let loaded = config(&f, vec![("p", vec![group(Some("nope"), &["*"], "out")])]);

        let expansion = expand(&loaded, &f.home, &f.state).unwrap();
        assert!(expansion.packages["p"].is_empty());
        assert_eq!(expansion.warnings.len(), 1);
    }

    #[test]
    fn first_matching_rename_applies() {
        let f = fixture(&["dot-zshrc", "dot-zshenv", "README"]);
        let rules = [("dot-zshrc", "zshrc"), ("dot-*", ".{1}")];
        let loaded = config(&f, vec![("p", vec![renamed(&["*"], &rules)])]);

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([
                (f.repo.join("out/zshrc"), f.repo.join("dot-zshrc")),
                (f.repo.join("out/.zshenv"), f.repo.join("dot-zshenv")),
                (f.repo.join("out/README"), f.repo.join("README")),
            ])
        );
    }

    #[test]
    fn rename_uses_tree_captures() {
        let f = fixture(&["dot-a", "dir/dot-b"]);
        let loaded = config(
            &f,
            vec![("p", vec![renamed(&["**/*"], &[("**/dot-*", "{1}.{2}")])])],
        );

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([
                (f.repo.join("out/.a"), f.repo.join("dot-a")),
                (f.repo.join("out/dir/.b"), f.repo.join("dir/dot-b")),
            ])
        );
    }

    #[test]
    fn rename_keeps_literal_braces() {
        let f = fixture(&["a"]);
        let loaded = config(&f, vec![("p", vec![renamed(&["a"], &[("a", "{{{0}}}")])])]);

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([(f.repo.join("out/{a}"), f.repo.join("a"))])
        );
    }

    #[test]
    fn rejects_renames_outside_target() {
        let f = fixture(&["a", "dot-"]);
        for (from, to) in [("a", "../a"), ("a", "/a"), ("a", ""), ("dot-*", ".{1}")] {
            let loaded = config(&f, vec![("p", vec![renamed(&["*"], &[(from, to)])])]);
            assert!(f.expand(&loaded).is_err(), "{from} -> {to}");
        }
    }

    #[test]
    fn rejects_invalid_rename_targets() {
        let f = fixture(&["a"]);
        for to in ["{2}", "{x}", "{1", "a}"] {
            let loaded = config(&f, vec![("p", vec![renamed(&["*"], &[("*", to)])])]);
            assert!(f.expand(&loaded).is_err(), "{to}");
        }
    }

    #[test]
    fn rejects_renames_to_one_target() {
        let f = fixture(&["a", "b"]);
        let loaded = config(&f, vec![("p", vec![renamed(&["*"], &[("*", "same")])])]);
        assert!(f.expand(&loaded).is_err());
    }

    #[test]
    fn warns_about_renames_that_rename_nothing() {
        let f = fixture(&["a"]);
        let rules = [("*", "{1}"), ("a", "b"), ("x", "y")];
        let loaded = config(&f, vec![("p", vec![renamed(&["*"], &rules)])]);

        let expansion = expand(&loaded, &f.home, &f.state).unwrap();
        assert_eq!(expansion.warnings.len(), 2, "{:?}", expansion.warnings);
    }

    fn tracked_fixture() -> Fixture {
        let f = fixture(&["a", "b"]);
        git(&f.repo, &["init", "-q"]);
        git(&f.repo, &["add", "a"]);
        f
    }

    #[test]
    fn git_tracked_defaults_to_true() {
        let f = tracked_fixture();
        let mut loaded = config(&f, vec![("p", vec![group(Some("."), &["*"], "out")])]);
        loaded.config.git_tracked = None;

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([(f.repo.join("out/a"), f.repo.join("a"))])
        );
    }

    #[test]
    fn git_tracked_is_overridden_by_package_then_link_group() {
        let f = tracked_fixture();
        let mut loaded = config(&f, vec![("p", vec![group(Some("."), &["*"], "out")])]);
        loaded.config.git_tracked = None;
        let package = loaded.config.packages.get_mut("p").unwrap();
        package.git_tracked = Some(false);
        assert_eq!(f.expand(&loaded).unwrap()["p"].len(), 2);

        let package = loaded.config.packages.get_mut("p").unwrap();
        package.links[0].git_tracked = Some(true);
        assert_eq!(f.expand(&loaded).unwrap()["p"].len(), 1);
    }

    #[test]
    fn git_tracked_off_links_untracked_files() {
        let f = tracked_fixture();
        let loaded = config(&f, vec![("p", vec![group(Some("."), &["*"], "out")])]);
        assert_eq!(f.expand(&loaded).unwrap()["p"].len(), 2);
    }

    #[test]
    fn git_tracked_fails_outside_a_repo() {
        let f = fixture(&["a"]);
        let mut loaded = config(&f, vec![("p", vec![group(Some("."), &["*"], "out")])]);
        loaded.config.git_tracked = None;
        assert!(f.expand(&loaded).is_err());
    }

    #[test]
    fn same_target_from_same_source_is_allowed() {
        let f = fixture(&["a.txt"]);
        let loaded = config(
            &f,
            vec![("p", vec![group(Some("."), &["*.txt", "a.*"], "out")])],
        );
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
                ("p", vec![group(Some("."), &["*.txt"], "out")]),
                ("q", vec![group(Some("."), &["*.txt"], "out")]),
            ],
        );
        assert!(f.expand(&loaded).is_err());
    }

    #[test]
    fn skips_state_dir_and_managed_links() {
        let f = fixture(&["a.txt", ".symsync.state/links/x"]);
        std::fs::create_dir_all(f.repo.join("out")).unwrap();
        std::os::unix::fs::symlink("../.symsync.state/links/x", f.repo.join("out/a.txt")).unwrap();

        let loaded = config(&f, vec![("p", vec![group(Some("."), &["**/*"], "out")])]);

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([(f.repo.join("out/a.txt"), f.repo.join("a.txt"))])
        );
    }

    #[test]
    fn skips_config_file() {
        let f = fixture(&["symsync.toml", "a.txt"]);
        let loaded = config(&f, vec![("p", vec![group(Some("."), &["*"], "out")])]);

        assert_eq!(
            f.expand(&loaded).unwrap()["p"],
            Targets::from([(f.repo.join("out/a.txt"), f.repo.join("a.txt"))])
        );
    }

    #[test]
    fn warns_about_globs_without_matches() {
        let f = fixture(&["a.txt"]);
        let loaded = config(
            &f,
            vec![("p", vec![group(Some("."), &["*.txt", "*.md"], "out")])],
        );

        let expansion = expand(&loaded, &f.home, &f.state).unwrap();
        assert_eq!(expansion.packages["p"].len(), 1);
        assert_eq!(expansion.warnings.len(), 1);
        assert_eq!(expansion.warnings[0].0, "p");
        assert!(
            expansion.warnings[0].1.contains("*.md"),
            "{:?}",
            expansion.warnings
        );
    }

    #[test]
    fn contained_paths_pass_and_others_fail() {
        let f = fixture(&["a.txt"]);
        let inside = Targets::from([(f.repo.join("out/a.txt"), f.repo.join("a.txt"))]);
        let packages = BTreeMap::from([("p".to_string(), inside)]);
        assert!(check_contained(&packages, &f.state, &f.repo, &f.home).is_ok());
        assert!(check_contained(&packages, &f.home.join("state"), &f.repo, &f.home).is_err());

        let outside = Targets::from([(f.home.join("a.txt"), f.repo.join("a.txt"))]);
        let packages = BTreeMap::from([("p".to_string(), outside)]);
        assert!(check_contained(&packages, &f.state, &f.repo, &f.home).is_err());
    }
}
