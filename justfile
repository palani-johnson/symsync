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

# print the JSON schema of the config
schema:
    cargo run --quiet -- gen-config-schema

##
## Nix recipes
##

# build the nix package
nix-build:
    nix build

# update the flake lock file
nix-update:
    nix flake update
