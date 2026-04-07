#![allow(unused_variables)]

fn basic_unsynced_dir() -> anyhow::Result<tempfile::TempDir> {
    let tmp_dir = tempfile::tempdir()?;
    let tmp_path = tmp_dir.path();

    // copy basic setup
    std::fs::copy("examples/basic/symsync.toml", tmp_path.join("symsync.toml"))?;
    std::fs::copy("examples/basic/file_0.txt", tmp_path.join("file_0.txt"))?;
    std::fs::copy("examples/basic/file_1.txt", tmp_path.join("file_1.txt"))?;
    std::fs::copy("examples/basic/file_2.txt", tmp_path.join("file_2.txt"))?;

    Ok(tmp_dir)
}

fn symsync_cmd_output<S: AsRef<std::ffi::OsStr>, I: IntoIterator<Item = S>>(
    args: I,
    working_dir: Option<&std::path::Path>,
) -> anyhow::Result<String> {
    let mut cmd = assert_cmd::Command::cargo_bin("symsync")?;

    cmd.args(args);
    if let Some(working_dir) = working_dir {
        cmd.current_dir(working_dir.canonicalize()?);
    }

    Ok(String::from_utf8_lossy(&cmd.ok()?.stdout).into_owned())
}

#[test]
fn verify_help_output() -> anyhow::Result<()> {
    let output = symsync_cmd_output(&["help"], None)?;

    anyhow::ensure!(
        output.starts_with("A symlink farm manager\n\nUsage: symsync"),
        "help output does not start with expected prefix"
    );

    Ok(())
}

const EXPECTED_UNSYNCED_PLAN: &str = "Symsync plan:

mkdir  ./out/
link   ./out/file_0.txt as ../.symsync.state/sources/064525aa0ae4a2240df5e54f08c3004cbe29cba1
link   ./out/file_1.txt as ../.symsync.state/sources/bb862d7ccad0c082b3db9f05b57d5f0384eb9471
link   ./out/file_2.txt as ../.symsync.state/sources/2b562f4c942af3f299539c934e5d23914c1cd43f
mkdir  ./.symsync.state/sources/
link   ./.symsync.state/sources/064525aa0ae4a2240df5e54f08c3004cbe29cba1 as ../../file_0.txt
link   ./.symsync.state/sources/bb862d7ccad0c082b3db9f05b57d5f0384eb9471 as ../../file_1.txt
link   ./.symsync.state/sources/2b562f4c942af3f299539c934e5d23914c1cd43f as ../../file_2.txt
write  ./.symsync.state/targets.toml";

fn expected_unsynched_plan_output() -> String {
    EXPECTED_UNSYNCED_PLAN.replace("/", std::path::MAIN_SEPARATOR_STR)
}

#[test]
fn verify_plan_from_unsynced() -> anyhow::Result<()> {
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
        anyhow::ensure!(
            expected == actual,
            "plan step {} does not match expected steps at index: '{}' != '{}'",
            index,
            expected,
            actual
        );
    }

    Ok(())
}

#[test]
fn verify_sync_output_from_unsynced() -> anyhow::Result<()> {
    let root_dir = basic_unsynced_dir()?;
    let output = symsync_cmd_output(&["sync"], Some(root_dir.path()))?;

    // Verify expected output
    for (index, (expected, actual)) in expected_unsynched_plan_output()
        .lines()
        .into_iter()
        .zip(output.lines().into_iter())
        .enumerate()
    {
        anyhow::ensure!(
            expected == actual,
            "plan step {} does not match expected steps at index: '{}' != '{}'",
            index,
            expected,
            actual
        );
    }

    // Verify result

    Ok(())
}
