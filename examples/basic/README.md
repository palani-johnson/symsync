# Basic example

This example links every file in `in/` to the same path in `out/`. The config sets `state_dir` and `link_type = "relative"`, so the state directory lives in this folder and every link is relative. The committed links work after a clone to any location.

The link chain for `out/a.txt` is `out/a.txt` → `.symsync.state/links/<sha256 of out/a.txt relative to the state directory>` → `in/a.txt`. To make it again, run `symsync sync` in this folder.
