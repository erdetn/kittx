#!/bin/sh
# Build/run KittxRecorder in release mode, or clean its Cargo build artifacts.
set -eu

usage() {
    cat <<'EOF'
Usage: build_recorder.sh [--build-only] [windows|linux|macos]
       build_recorder.sh clean

Build KittxRecorder in release mode. With no argument, use the active Rust
compiler's exact host target. An OS argument retains the host CPU architecture
(x86_64 or aarch64) and selects Windows MSVC, Linux GNU, or macOS.

Run the application after a successful build when the selected target equals the
compiler's host target. Foreign targets are built only. --build-only also skips
launching native builds. Options may appear before or after the OS argument.

clean removes the recorder's Cargo build artifacts without building or launching.
Use clean by itself. Cargo target-directory settings also apply to cleaning.

Requires Rust 1.92 or newer and the target's native dependencies, linker, and SDK.
Use a POSIX shell such as Git Bash on Windows. Toolchains are not installed here.
EOF
}

requested_os=
build_only=false
clean=false
for argument in "$@"; do
    case "$argument" in
        -h|--help) usage; exit 0 ;;
        --build-only) build_only=true ;;
        clean)
            if [ "$#" -ne 1 ]; then
                printf 'Use clean by itself.\n' >&2
                usage >&2
                exit 2
            fi
            clean=true ;;
        windows|linux|macos)
            if [ -n "$requested_os" ]; then
                printf 'Specify only one operating system.\n' >&2
                usage >&2
                exit 2
            fi
            requested_os=$argument ;;
        *) printf 'Unsupported argument: %s\n' "$argument" >&2
           usage >&2
           exit 2 ;;
    esac
done

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
package_dir="$script_dir/src/kittxrecorder"
cd "$package_dir"

if [ "$clean" = true ]; then
    printf 'Cleaning KittxRecorder build artifacts\n'
    exec cargo clean --manifest-path "$package_dir/Cargo.toml"
fi

# Resolve the compiler in the package directory, including any toolchain override.
compiler_info=$(rustc -vV)
host=$(printf '%s\n' "$compiler_info" | sed -n 's/^host: //p' | tr -d '\r')
if [ -z "$host" ]; then
    printf 'Cannot determine the host target from rustc -vV.\n' >&2
    exit 1
fi

if [ -z "$requested_os" ]; then
    case "$host" in
        *-apple-darwin|*-pc-windows-*|*-unknown-linux-gnu|*-unknown-linux-musl)
            target=$host ;;
        *) printf 'Unsupported desktop host: %s\n' "$host" >&2
           exit 1 ;;
    esac
else
    architecture=${host%%-*}
    case "$architecture" in
        x86_64|aarch64) ;;
        *) printf 'OS selection supports x86_64 and aarch64; found %s.\n' "$architecture" >&2
           exit 1 ;;
    esac
    case "$requested_os" in
        windows) target="$architecture-pc-windows-msvc" ;;
        linux) target="$architecture-unknown-linux-gnu" ;;
        macos) target="$architecture-apple-darwin" ;;
    esac
fi

printf 'Building KittxRecorder in release mode for %s\n' "$target"
if cargo build --locked --release --bin KittxRecorder \
    --manifest-path "$package_dir/Cargo.toml" --target "$target"; then
    printf 'KittxRecorder release build completed for %s\n' "$target"
else
    status=$?
    printf 'Build failed. Check that the selected Rust target, native dependencies, linker, and OS SDK are installed.\n' >&2
    exit "$status"
fi

if [ "$build_only" = true ]; then
    printf 'Build only; application was not started.\n'
elif [ "$target" != "$host" ]; then
    printf 'Target %s differs from host %s; run this build on the target platform.\n' "$target" "$host"
else
    printf 'Starting KittxRecorder for %s\n' "$target"
    # Cargo locates the executable even with a custom target directory. This
    # reuses the completed release build and propagates the program's exit status.
    exec cargo run --locked --release --bin KittxRecorder \
        --manifest-path "$package_dir/Cargo.toml" --target "$target"
fi
