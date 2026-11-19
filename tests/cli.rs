use std::path::{Path, PathBuf};

use assert_cmd::assert::Assert;
use assert_cmd::cargo::cargo_bin_cmd;

struct Sandbox {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    Sandbox {
        _tmp: tmp,
        state: root.join("state"),
        home,
        root,
    }
}

const ZSH_CONFIG: &str = r#"
[[packages.zsh.links]]
root = "zsh"
sources = ["*"]
target = "~"
hidden = true
"#;

impl Sandbox {
    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).unwrap()
    }

    /// A repo with a zsh package that links `.zshrc` and `.zshenv` into the home directory.
    fn zsh_repo(&self, repo: &str) {
        self.write(&format!("{repo}/symsync.toml"), ZSH_CONFIG);
        self.write(&format!("{repo}/zsh/.zshrc"), "zshrc");
        self.write(&format!("{repo}/zsh/.zshenv"), "zshenv");
    }

    fn run(&self, dir: &str, args: &[&str]) -> Assert {
        cargo_bin_cmd!("symsync")
            .current_dir(self.path(dir))
            .env("HOME", &self.home)
            .env("SYMSYNC_STATE_DIR", &self.state)
            .env_remove("SYMSYNC_LINK_TYPE")
            .env_remove("XDG_STATE_HOME")
            .args(args)
            .assert()
    }
}

fn stdout(assert: &Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

fn is_link(path: &Path) -> bool {
    path.is_symlink()
}

#[test]
fn plan_changes_nothing() {
    let s = sandbox();
    s.zsh_repo("repo");

    let out = stdout(&s.run("repo", &["plan"]).success());
    assert!(out.contains("link    ~/.zshrc"), "{out}");
    assert!(!s.state.exists());
    assert!(!is_link(&s.path("home/.zshrc")));
}

#[test]
fn sync_links_through_state_dir() {
    let s = sandbox();
    s.zsh_repo("repo");
    s.run("repo", &["sync"]).success();

    assert_eq!(s.read("home/.zshrc"), "zshrc");
    let value = std::fs::read_link(s.path("home/.zshrc")).unwrap();
    assert!(
        value.starts_with(s.state.join("links")),
        "{}",
        value.display()
    );

    let out = stdout(&s.run("repo", &["sync"]).success());
    assert!(out.contains("already in sync"), "{out}");
}

#[test]
fn edits_go_both_ways() {
    let s = sandbox();
    s.zsh_repo("repo");
    s.run("repo", &["sync"]).success();

    std::fs::write(s.path("home/.zshrc"), "edited in app").unwrap();
    assert_eq!(s.read("repo/zsh/.zshrc"), "edited in app");
}

#[test]
fn deleted_source_is_unlinked() {
    let s = sandbox();
    s.zsh_repo("repo");
    s.run("repo", &["sync"]).success();

    std::fs::remove_file(s.path("repo/zsh/.zshenv")).unwrap();
    let out = stdout(&s.run("repo", &["sync"]).success());
    assert!(out.contains("unlink  ~/.zshenv"), "{out}");
    assert!(!is_link(&s.path("home/.zshenv")));
    assert!(is_link(&s.path("home/.zshrc")));
}

#[test]
fn conflict_stops_sync_before_any_change() {
    let s = sandbox();
    s.zsh_repo("repo");
    s.write("home/.zshenv", "mine");

    s.run("repo", &["sync"]).failure();
    assert!(!is_link(&s.path("home/.zshrc")));
    assert_eq!(s.read("home/.zshenv"), "mine");
}

#[test]
fn drift_is_reported_and_adopted() {
    let s = sandbox();
    s.zsh_repo("repo");
    s.run("repo", &["sync"]).success();

    // An atomic save replaces the link with a regular file.
    std::fs::remove_file(s.path("home/.zshrc")).unwrap();
    s.write("home/.zshrc", "saved by app");

    let out = stdout(&s.run("repo", &["status"]).success());
    assert!(out.contains("drifted"), "{out}");
    s.run("repo", &["sync"]).failure();

    s.run("home", &["adopt", ".zshrc"]).success();
    assert_eq!(s.read("repo/zsh/.zshrc"), "saved by app");
    assert!(is_link(&s.path("home/.zshrc")));
    let out = stdout(&s.run("repo", &["sync"]).success());
    assert!(out.contains("already in sync"), "{out}");
}

#[test]
fn moved_repo_keeps_target_links() {
    let s = sandbox();
    s.zsh_repo("repo");
    s.run("repo", &["sync"]).success();
    let before = std::fs::read_link(s.path("home/.zshrc")).unwrap();

    std::fs::rename(s.path("repo"), s.path("moved")).unwrap();
    let out = stdout(&s.run("moved", &["sync"]).success());
    assert!(out.contains("moved from"), "{out}");

    assert_eq!(std::fs::read_link(s.path("home/.zshrc")).unwrap(), before);
    assert_eq!(s.read("home/.zshrc"), "zshrc");
}

#[test]
fn same_package_in_two_repos_is_a_conflict() {
    let s = sandbox();
    s.zsh_repo("one");
    s.zsh_repo("two");
    s.run("one", &["sync"]).success();

    s.write("two/zsh/.zshrc", "other zshrc");
    s.run("two", &["sync"]).failure();
    assert_eq!(s.read("home/.zshrc"), "zshrc");
}

#[test]
fn packages_from_many_repos_coexist() {
    let s = sandbox();
    s.zsh_repo("one");
    s.write(
        "two/symsync.toml",
        r#"
[[packages.git.links]]
root = "git"
sources = ["*"]
target = "~/.config/git"
"#,
    );
    s.write("two/git/config", "git");

    s.run("one", &["sync"]).success();
    s.run("two", &["sync"]).success();
    assert_eq!(s.read("home/.zshrc"), "zshrc");
    assert_eq!(s.read("home/.config/git/config"), "git");

    // Removing a package from one repo must not touch the other repo.
    s.write("two/symsync.toml", "packages = {}");
    s.run("two", &["sync"]).success();
    assert!(!is_link(&s.path("home/.config/git/config")));
    assert!(is_link(&s.path("home/.zshrc")));
}

#[test]
fn orphaned_package_can_be_removed() {
    let s = sandbox();
    s.zsh_repo("repo");
    s.run("repo", &["sync"]).success();
    std::fs::remove_dir_all(s.path("repo")).unwrap();

    let out = stdout(&s.run("home", &["status"]).success());
    assert!(out.contains("zsh (orphaned"), "{out}");

    s.run("home", &["remove", "zsh"]).success();
    assert!(!is_link(&s.path("home/.zshrc")));
    assert!(!is_link(&s.path("home/.zshenv")));
    let out = stdout(&s.run("home", &["status"]).success());
    assert!(out.contains("no packages"), "{out}");
}

#[test]
fn relative_chain_inside_repo_survives_a_copy() {
    let s = sandbox();
    s.write(
        "repo/symsync.toml",
        r#"
state_dir = ".symsync.state"
link_type = "relative"

[[packages.files.links]]
root = "in"
sources = ["*.txt"]
target = "out"
"#,
    );
    s.write("repo/in/a.txt", "a");

    let sync = |dir: &str| {
        let assert = cargo_bin_cmd!("symsync")
            .current_dir(s.path(dir))
            .env("HOME", &s.home)
            .env_remove("SYMSYNC_STATE_DIR")
            .env_remove("SYMSYNC_LINK_TYPE")
            .env_remove("XDG_STATE_HOME")
            .arg("sync")
            .assert()
            .success();
        stdout(&assert)
    };

    sync("repo");
    assert_eq!(s.read("repo/out/a.txt"), "a");

    std::fs::rename(s.path("repo"), s.path("copy")).unwrap();
    assert_eq!(s.read("copy/out/a.txt"), "a");

    let packages = s.read("copy/.symsync.state/packages.toml");
    assert!(!packages.contains(s.root.to_str().unwrap()), "{packages}");

    let out = sync("copy");
    assert!(out.contains("already in sync"), "{out}");
}
