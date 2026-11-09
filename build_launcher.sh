#!/bin/sh
# Build KittxLauncher with the four UI libraries linked into its process.
set -eu
case "${1-}" in
    '') build_only=false ;;
    --build-only) build_only=true ;;
    -h|--help)
        printf 'Usage: build_launcher.sh [--build-only]\nBuild KittxLauncher with all four integrated UIs for this desktop. Requires Rust 1.92+.\n'
        exit 0 ;;
    *) printf 'Unsupported argument: %s\n' "$1" >&2; exit 2 ;;
esac
if [ "$#" -gt 1 ]; then
    printf 'Usage: build_launcher.sh [--build-only]\n' >&2
    exit 2
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
cd "$script_dir"
target_directory=${CARGO_TARGET_DIR:-"$script_dir/src/kittxlauncher/target"}
cargo build --locked --release --manifest-path src/kittxlauncher/Cargo.toml \
    --target-dir "$target_directory" --bin KittxLauncher
if [ "$build_only" = false ]; then
    exec cargo run --locked --release --manifest-path src/kittxlauncher/Cargo.toml \
        --target-dir "$target_directory" --bin KittxLauncher
fi
