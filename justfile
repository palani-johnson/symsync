[default]
_list:
    @just --list

##
## Cargo recipes
##

# build a debug binary
build:
    cargo build

# run the tests
test *args:
    cargo test {{ args }}

# format the code
fmt:
    cargo fmt

# run clippy
lint:
    cargo clippy --all-targets -- -D warnings

# run the same checks as CI
ci:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# install symsync from this clone
install:
    cargo install --path .

# run symsync from source
run *args:
    cargo run -- {{ args }}

# build every JSON schema in schemas/
schemas:
    cargo run --quiet -- gen-config-schema > schemas/symsync.schema.json

##
## Nix recipes
##

# build the nix package
nix-build:
    nix build

# update the flake lock file
nix-update:
    nix flake update

##
## GitHub Actions recipes
##

# lock workflow actions after a change to the workflows
actions-lock:
    gh actions-lock --no-narrow

# re-resolve moved tags and update the actions lock file
actions-relock:
    gh actions-lock --no-narrow --relock

# check that the actions lock file is up to date
actions-verify:
    gh actions-lock --verify
