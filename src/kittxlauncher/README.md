# KittxLauncher

A native desktop launcher with **View**, **Record**, **Analyze**, and
**Real-Time Probe** buttons. Each opens the corresponding subproject's UI as a
window in the launcher's own process and GPUI event loop. No external executable
is started. Only one tool window may be open; the launcher stays available with
its tool buttons disabled. Closing the tool returns focus and menus to the launcher.
Recorder and Probe finish their normal stop/finalization behavior before closing.
Quit/Exit from a tool closes that tool. Closing the launcher while a tool is open
requests that tool's normal close flow and returns to the launcher; quit again
from the launcher to exit the application.

The four subprojects remain separate Cargo packages with reusable `ui` modules
and their existing standalone binary targets. The root ELM327 library and binary
remain separate too. This does not turn the repository into a single-binary project.

From the repository root:

```sh
./build_launcher.sh
# Or:
cargo run --locked --manifest-path src/kittxlauncher/Cargo.toml
```

Use `./build_launcher.sh --build-only` to build without opening windows. The script
builds only KittxLauncher, linking the four UI libraries as dependencies. It does
not require or build the standalone tool executables. The resulting executable
can run without sibling tool binaries or a source checkout. Requires Rust 1.92+
and the same native GPUI dependencies as the standalone desktop projects. Use a
POSIX shell such as Git Bash on Windows. `CARGO_TARGET_DIR` is respected.

Serial support is enabled by default. For TCP only:

```sh
cargo run --locked --manifest-path src/kittxlauncher/Cargo.toml --no-default-features --features ui
```

KittxLauncher and all four standalone entry points acquire the same exclusive OS
file lock before creating any UI. This prevents a standalone recorder/probe or a
second launcher from competing with the integrated application, including while
the launcher is idle. Embedded windows use the host's lock. Locks live in
`$HOME/.kittx` on macOS/Linux and `%LOCALAPPDATA%/.kittx` on Windows. Conflicts
report an error on stderr and exit with status 1. The OS releases locks after
normal exit or a crash; lock files remain on disk and must not be deleted while
the application runs. External programs and older binaries without these guards
do not participate in this per-user coordination.

Each subproject exposes `ui::open` and `ui::close` for hosts and `ui::run` for its
standalone entry point. Initialize GPUI once before opening tool windows. The
host owns the event loop and exclusivity between tools. Closing a tool never
quits an event loop that still has a host window. Key bindings are scoped to the
tool and registered once; each visit restores that tool's native menu.

Validation:

```sh
cargo test --locked --manifest-path src/kittxlauncher/Cargo.toml --features ui-test
cargo test --locked --manifest-path src/kittxlauncher/Cargo.toml --no-default-features --features ui-test
cargo clippy --locked --manifest-path src/kittxlauncher/Cargo.toml --all-targets --features ui-test -- -D warnings
```

Headless tests open the real four tool UIs in one host, repeatedly close/reopen
and switch tools, check keyboard Quit and Exit, reject concurrent windows, and
preserve the host. Recorder and Probe test finalization with both standalone
and hosted windows using local TCP emulators. Shared lock tests check contention
and crash release. Normal automated tests require no serial hardware.
