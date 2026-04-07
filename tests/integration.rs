use rstest::*;

#[fixture]
fn root_dir() -> tempfile::TempDir {
    let tmp_dir = tempfile::tempdir().unwrap();
    let tmp_path = tmp_dir.path();

    // copy basic setup here
    std::fs::copy("examples/basic/symsync.toml", tmp_path.join("symsync.toml")).unwrap();
    std::fs::copy("examples/basic/file.txt", tmp_path.join("file.txt")).unwrap();
    std::fs::copy("examples/basic/file2.txt", tmp_path.join("file2.txt")).unwrap();

    // move to this dir
    std::env::set_current_dir(tmp_path).unwrap();

    tmp_dir
}

#[rstest]
#[allow(unused)]
fn test_plan(root_dir: tempfile::TempDir) {
    // check that all our paths exist
    assert!(std::path::Path::new("symsync.toml").exists());
    assert!(std::path::Path::new("file.txt").exists());
    assert!(std::path::Path::new("file2.txt").exists());
}
