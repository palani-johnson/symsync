#![allow(non_snake_case)]

use anyhow::{Context, *};
use rstest::*;

// Helper functions

const CONFIG_TEXT: &str = r#"
[[entries]]
sources = ["*.txt"]
target = "target"
target_link_type = "relative"
"#;

const SOURCE_FILES: &[&str] = &["file_0.txt", "file_1.txt", "file_2.txt"];

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

enum Step {
    Unsynced,
    FirstPlan,
    FirstSync,
    RemovedFile,
}
struct StateOutput {
    root_dir: tempfile::TempDir,
    outputs: Vec<StepOutput>,
}

struct StepOutput {
    stdout: String,

    #[allow(unused)]
    step: Step,
}

impl StateOutput {
    fn latest_output(&self) -> Result<&StepOutput> {
        self.outputs
            .first()
            .ok_or(anyhow!("No latest output".to_owned()))
    }
}

impl Step {
    fn setup(&self) -> Result<StateOutput> {
        match self {
            Step::Unsynced => {
                let root_dir = tempfile::tempdir().with_context(|| "failed to create tempdir")?;
                let tmp_path = root_dir.path();

                // copy basic setup
                std::fs::write(tmp_path.join("symsync.toml"), CONFIG_TEXT)
                    .with_context(|| "failed to write symsync.toml file")?;

                for file in SOURCE_FILES {
                    std::fs::write(tmp_path.join(file), file)
                        .with_context(|| "failed to write source file")?;
                }

                Ok(StateOutput {
                    root_dir,
                    outputs: Vec::new(),
                })
            }
            Step::FirstPlan => {
                let StateOutput {
                    root_dir,
                    mut outputs,
                } = Step::Unsynced.setup()?;

                outputs.push(StepOutput {
                    stdout: symsync_cmd_output(&["plan"], Some(root_dir.path()))?,
                    step: Step::FirstPlan,
                });

                Ok(StateOutput { root_dir, outputs })
            }
            Step::FirstSync => {
                let StateOutput {
                    root_dir,
                    mut outputs,
                } = Step::Unsynced.setup()?;

                outputs.push(StepOutput {
                    stdout: symsync_cmd_output(&["sync"], Some(root_dir.path()))?,
                    step: Step::FirstSync,
                });

                Ok(StateOutput { root_dir, outputs })
            }
            Step::RemovedFile => {
                let StateOutput {
                    root_dir,
                    mut outputs,
                } = Step::FirstSync.setup()?;

                std::fs::remove_file(root_dir.path().join("file_0.txt"))
                    .with_context(|| "failed to remove file_0.txt")?;

                outputs.push(StepOutput {
                    stdout: symsync_cmd_output(&["sync"], Some(root_dir.path()))?,
                    step: Step::FirstSync,
                });

                Ok(StateOutput { root_dir, outputs })
            }
        }
    }
}

// Verify stateless commands

#[rstest]
fn verify_help_output() -> Result<()> {
    let output = symsync_cmd_output(&["help"], None)?;

    ensure!(
        output.starts_with("A symlink farm manager\n\nUsage: symsync"),
        "help output does not start with expected prefix"
    );

    Ok(())
}

// Verify outputs

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

#[rstest]
#[case::FirstPlan(Step::FirstPlan)]
fn verify_plan_output(#[case] step: Step) -> Result<()> {
    let step_result = step.setup()?;
    let output = step_result.latest_output()?;

    for (index, (expected, actual)) in expected_unsynched_plan_output()
        .lines()
        .into_iter()
        .zip(output.stdout.lines().into_iter())
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
#[case::FirstSync(Step::FirstSync)]
fn verify_sync_output(#[case] step: Step) -> Result<()> {
    let step_result = step.setup()?;
    let output = step_result.latest_output()?;

    for (index, (expected, actual)) in expected_unsynched_plan_output()
        .lines()
        .into_iter()
        .zip(output.stdout.lines().into_iter())
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

// Verify sources

#[rstest]
#[case::FirstSync__file_0(Step::FirstSync, "file_0.txt")]
#[case::FirstSync__file_1(Step::FirstSync, "file_1.txt")]
#[case::FirstSync__file_2(Step::FirstSync, "file_2.txt")]
#[case::RemovedFile__file_1(Step::RemovedFile, "file_1.txt")]
#[case::RemovedFile__file_2(Step::RemovedFile, "file_2.txt")]
fn verify_sources(#[case] step: Step, #[case] file: &str) -> Result<()> {
    let step_result = step.setup()?;

    let path = step_result.root_dir.path().join(file);
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
#[case::FirstSync__file_0(Step::FirstSync, "file_0.txt")]
#[case::FirstSync__file_1(Step::FirstSync, "file_1.txt")]
#[case::FirstSync__file_2(Step::FirstSync, "file_2.txt")]
#[case::RemovedFile__file_1(Step::RemovedFile, "file_1.txt")]
#[case::RemovedFile__file_2(Step::RemovedFile, "file_2.txt")]
fn verify_target_content_matches_source(#[case] step: Step, #[case] file: &str) -> Result<()> {
    let step_result = step.setup()?;

    let source_path = step_result.root_dir.path().join(file);
    let target_path = step_result.root_dir.path().join("target").join(file);

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
