#![allow(unused_variables)]

use anyhow::{Context, *};
use rstest::*;

const CONFIG_TEXT: &str = r#"
[[entries]]
sources = ["*.txt"]
target = "target"
target_link_type = "relative"
"#;

const SOURCE_FILES: &[&str] = &["file_0.txt", "file_1.txt", "file_2.txt"];

fn basic_unsynced_dir() -> Result<tempfile::TempDir> {
    let tmp_dir = tempfile::tempdir().with_context(|| "failed to create tempdir")?;
    let tmp_path = tmp_dir.path();

    // copy basic setup
    std::fs::write(tmp_path.join("symsync.toml"), CONFIG_TEXT)
        .with_context(|| "failed to write symsync.toml file")?;

    for file in SOURCE_FILES {
        std::fs::write(tmp_path.join(file), file).with_context(|| "failed to write source file")?;
    }

    Ok(tmp_dir)
}

fn symsync_cmd_output<S: AsRef<std::ffi::OsStr>, I: IntoIterator<Item = S>>(
    args: I,
    working_dir: Option<&std::path::Path>,
) -> Result<String> {
    let mut cmd = assert_cmd::Command::cargo_bin("symsync")?;

    cmd.args(args);
    if let Some(working_dir) = working_dir {
        cmd.current_dir(working_dir.canonicalize()?);
    }

    Ok(String::from_utf8_lossy(&cmd.ok()?.stdout).into_owned())
}

#[test]
fn verify_help_output() -> Result<()> {
    let output = symsync_cmd_output(&["help"], None)?;

    ensure!(
        output.starts_with("A symlink farm manager\n\nUsage: symsync"),
        "help output does not start with expected prefix"
    );

    Ok(())
}

const EXPECTED_UNSYNCED_PLAN: &str = "Symsync plan:

mkdir  ./target/
link   ./target/file_0.txt as ../.symsync.state/sources/064525aa0ae4a2240df5e54f08c3004cbe29cba1
link   ./target/file_1.txt as ../.symsync.state/sources/bb862d7ccad0c082b3db9f05b57d5f0384eb9471
link   ./target/file_2.txt as ../.symsync.state/sources/2b562f4c942af3f299539c934e5d23914c1cd43f
mkdir  ./.symsync.state/sources/
link   ./.symsync.state/sources/064525aa0ae4a2240df5e54f08c3004cbe29cba1 as ../../file_0.txt
link   ./.symsync.state/sources/bb862d7ccad0c082b3db9f05b57d5f0384eb9471 as ../../file_1.txt
link   ./.symsync.state/sources/2b562f4c942af3f299539c934e5d23914c1cd43f as ../../file_2.txt
write  ./.symsync.state/targets.toml";

fn expected_unsynched_plan_output() -> String {
    EXPECTED_UNSYNCED_PLAN.replace("/", std::path::MAIN_SEPARATOR_STR)
}

#[test]
fn verify_plan_from_unsynced() -> Result<()> {
    let root_dir = basic_unsynced_dir()?;
    let output = symsync_cmd_output(&["plan"], Some(root_dir.path()))?;

    // Verify expected output
    for (index, (expected, actual)) in expected_unsynched_plan_output()
        .lines()
        .into_iter()
        .map(|s| s.replace("/", std::path::MAIN_SEPARATOR_STR))
        .zip(output.lines().into_iter())
        .enumerate()
    {
        ensure!(
            expected == actual,
            "plan step {} does not match expected steps at index: '{}' != '{}'",
            index,
            expected,
            actual
        );
    }

    Ok(())
}

type SyncResult = Result<(String, tempfile::TempDir)>;

#[fixture]
#[once]
fn basic_sync() -> SyncResult {
    let root_dir = basic_unsynced_dir()?;
    let output = symsync_cmd_output(&["sync"], Some(root_dir.path()))?;

    Ok((output, root_dir))
}

#[rstest]
fn verify_sync_output_from_unsynced(basic_sync: &SyncResult) -> Result<()> {
    let (output, _) = basic_sync.as_ref().map_err(|e| anyhow!(e.to_string()))?;

    // Verify expected output
    for (index, (expected, actual)) in expected_unsynched_plan_output()
        .lines()
        .into_iter()
        .zip(output.lines().into_iter())
        .enumerate()
    {
        ensure!(
            expected == actual,
            "plan step {} does not match expected steps at index: '{}' != '{}'",
            index,
            expected,
            actual
        );
    }

    Ok(())
}

#[rstest]
fn verify_sync_from_unsynced_sources(
    basic_sync: &SyncResult,
    #[values("file_0.txt", "file_1.txt", "file_2.txt")] file: &str,
) -> Result<()> {
    let (output, root_dir) = basic_sync.as_ref().map_err(|e| anyhow!(e.to_string()))?;

    let path = root_dir.path().join(file);
    ensure!(
        path.exists(),
        "source file '{}' does not exist after sync",
        path.display()
    );

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("error while reading source file {}", path.display()))?;

    ensure!(
        content == file.to_owned(),
        "source file '{}' content does not match after sync",
        path.display()
    );

    Ok(())
}

#[rstest]
fn verify_sync_from_unsynced_targets_content_match_source(
    basic_sync: &SyncResult,
    #[values("file_0.txt", "file_1.txt", "file_2.txt")] file: &str,
) -> Result<()> {
    let (output, root_dir) = basic_sync.as_ref().map_err(|e| anyhow!(e.to_string()))?;

    let source_path = root_dir.path().join(file);
    let target_path = root_dir.path().join("target").join(file);

    let source_content = std::fs::read_to_string(&source_path)
        .with_context(|| format!("error while reading source file {}", source_path.display()))?;
    let target_content = std::fs::read_to_string(&target_path)
        .with_context(|| format!("error while reading target file {}", target_path.display()))?;

    ensure!(
        source_content == target_content,
        "target file '{}' content does not match source file",
        target_path.display()
    );

    Ok(())
}
