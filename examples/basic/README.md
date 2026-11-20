# Basic example

This example links every git-tracked file in `in/` to the same path in `out/`. The link group has no `sources`, so it uses the default, `**/*`. The config sets `state_dir` and `link_type = "relative"`, so the state directory lives in this folder and every link is relative. The committed links work after a clone to any location.

The link chain for `out/a.txt` is `out/a.txt` → `.symsync.state/links/<sha256 of out/a.txt relative to the state directory>` → `in/a.txt`. To make it again, run `symsync sync` in this folder.

## Updating

After a change to symsync or to this example, run this command in this folder:

```sh
just update
```

From the repo root, run `just examples/basic/update`.

It removes `.symsync.state` and `out`, then runs `sync` from source in this folder. The state directory and links then match the current code. Unset environment variables such as `SYMSYNC_STATE_DIR` do not change the result, because the recipe removes them. Commit the result together with the change.
