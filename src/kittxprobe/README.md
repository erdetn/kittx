# KittxProbe

A native desktop recorder with live PID plots and DTC reads. The connection and
capture controls are based on KittxRecorder; acquisition uses its existing worker.
Read-only ViewerUI models decode and plot committed data, including all 560 numeric
fields from the supported PIDs in [PIDS.MD](../../PIDS.MD). The library's synchronous
ELM327 API and version-1 `.kittx` schema are unchanged.

## Run

Use [KittxLauncher](../kittxlauncher/README.md) to open this UI inside its process.
This package retains its standalone binary and exposes `ui::open`, `ui::close`,
and `ui::run` behind the `ui` feature.
Only one Kittx UI may run per user, including direct launches. Closing Probe
keeps its stop/save prompts before returning to the launcher.

```sh
cargo run --locked --release --manifest-path src/kittxprobe/Cargo.toml
./build_probe.sh
./build_probe.sh --build-only
./build_probe.sh windows
./build_probe.sh linux
./build_probe.sh macos
./build_probe.sh clean
```

The executable is **KittxProbe** (`KittxProbe.exe` on Windows). Direct Cargo builds
put it in `src/kittxprobe/target/release/`; the script uses
`src/kittxprobe/target/<host-triple>/release/`. Cargo's target-directory settings
can override these paths. The script also accepts `windows`, `linux` or `macos`,
retaining the host CPU architecture. Foreign targets build without launching and
require their toolchain/native dependencies. Use a POSIX shell or Git Bash.

With no arguments, the script builds for the compiler's exact host target and
runs KittxProbe after a successful build. **clean** runs Cargo clean using the
probe manifest, removes build artifacts from its configured target directory,
and exits without building or launching. Recordings outside that target directory
are untouched.
Use `clean` alone; it cannot be combined with an OS or `--build-only`.

The isolated package uses Rust 1.92+ and GPUI Kit 0.7.3. See the
[viewer platform dependencies](../viewerui/README.md#build-and-run).
Default features enable desktop serial and UI. A TCP-only desktop build is:

```sh
cargo run --locked --release --manifest-path src/kittxprobe/Cargo.toml --no-default-features --features ui
```

## Capture, inspect, save

1. In the scrollable side panel, choose **TCP/IP** (host and port) or **Serial**.
   Serial uses the recorder's detected/custom port combobox, **Refresh ports**,
   editable baud-rate combobox, 8N1 and no flow control. macOS callout-device
   handling is the same as the recorder. These selectors do not open a port.
2. Check the desired PIDs in the searchable measurement combobox. Bulk controls
   select decoded measurements, all PIDs, or none. Check stored (03), pending (07)
   and/or permanent (0A) DTCs independently. A documented raw manufacturer read
   profile is also available, as in the recorder. DTC-only and measurement-only
   captures are supported. Only selected requests are scheduled.
3. Click **Start**. The same button changes to **Stop** during capture.
   Capture begins in a unique temporary working directory; there
   is no save dialog before connecting. Settings lock until the worker stops.
   The adapter is initialized, VIN is read, PID support is discovered, and selected
   initial DTC scans run. The recorder's existing valid/consistent VIN requirement
   applies. An optional fixed-time mode is available in **Recording mode**.
4. Numeric measurements appear as they are committed. Equal unit strings share a
   track and Y scale; each ECU/connection remains a separately labeled curve.
   Every plot has a 160-pixel drawing area, and the tracks scroll vertically.
   Legends show names, ECU/segment identity, and the latest visible value. Live
   plots automatically show the most recent 60 seconds; after Stop, they fit the
   whole session. Use **Zoom + / Zoom −**, or the mouse wheel over a drawing, to
   zoom the shared time axis. Wheel zoom anchors at the pointer. Drag a drawing
   or use **Earlier / Later** to pan. All unit tracks stay aligned, with separate
   Y scales. Zooming reloads original samples for the chosen interval, including
   history outside the rolling minute. The chosen interval stays fixed during
   capture and after Stop. **Follow live** resumes the rolling view while running;
   **Fit session** resets to the full interval after Stop. Scroll over labels or
   gaps between drawings to move through the unit tracks.
   Different units always occupy different tracks. Unsupported/status-only data
   remains raw. The full raw payload and all acquired samples stay in the file.
5. The DTC panel displays the latest 200 reads across the session, newest first,
   including category, sender, observation time, codes and outcome. Successful
   empty results, unsupported/failed reads and raw results remain distinct.
   Only selected DTC categories are acquired and displayed. DTCs run initially,
   at the chosen interval, and during Stop. Read times do not establish fault onset.
6. Click **Stop**. The worker finishes current/preparation operations, runs selected
   final DTC reads, commits and disconnects. **Save .kittx…** becomes available once
   the worker has stopped. Choose a new filename; its extension is normalized to
   `.kittx`. Existing files are rejected. Saving uses SQLite backup so committed
   WAL contents are included in a standalone file that opens in ViewerUI or
   KittxAnalyzer. Recording metadata identifies the producer as `KittxProbe`.
7. Cancelling or failing Save preserves the working recording and allows retry.
   Saving is optional: starting again creates a new capture and retains the
   previous working file; the footer shows the most recent unsaved previous path.
   Exiting during capture first
   requests Stop and waits for finalization. Unsaved working files and diagnostic
   logs remain at the path shown in the footer, under the OS temporary directory;
   save them to a permanent location before OS temporary-file cleanup. Files in
   an unfinished state retain their WAL sidecars and require the existing explicit
   recovery workflow before export. The app performs no automatic repair.

The footer reports VIN, acquisition/commit/failure counts, the working path, saved
path and diagnostic log. Recorder logging is reused, including
`KITTX_RECORDER_LOG=debug` when full wire diagnostics are needed. Optional vehicle
metadata and the light/dark theme controls are inherited from the recorder.
The default dark theme uses muted navy backgrounds, pastel blue accents and
cool white text across panels, inputs, menus and plot controls.
Keyboard/menu: **Ctrl/Cmd+R** Start, **Ctrl/Cmd+Shift+R** Stop,
**Ctrl/Cmd+S** Save, **Ctrl/Cmd+Q** Quit.

## Scheduling and bounds

One application-owned recorder worker processes adapter requests sequentially.
The UI checks its latest status every 100 ms and starts a read-only plot refresh
at most every 500 ms when commits change. There is one background plot read and
one coalesced latest request; a generation check rejects a previous session's
result. A final refresh follows Stop. Plot reads and disk exports stay off the UI
thread. Sample queries use a frozen committed cutoff and the viewer's bounded
envelope/continuity rules, up to 4096 numeric series and approximately 96,000 plot
points plus viewport neighbors. Plot refresh latency depends on recording size
and the selected PIDs; a 500 ms refresh interval is not a guaranteed frame rate.
Before disconnect/finalization, the worker closes the live-reader gate and waits
for the current snapshot to release all SQLite connections. New live reads are
rejected until a fresh stopped snapshot is requested. This also protects fixed-time
completion and failure cleanup from a live reader blocking WAL finalization.
The latest 200 DTC reads and up to 1000 codes per read are displayed; all original
reads remain in the file. Save has a 60-second deadline and keeps the source on
failure. No automatic DTC clearing, adapter retry or chip reset is introduced.

## Verification

```sh
cargo fmt --manifest-path src/kittxprobe/Cargo.toml --check
cargo test --locked --manifest-path src/kittxprobe/Cargo.toml --features ui-test
cargo test --locked --manifest-path src/kittxprobe/Cargo.toml --no-default-features
cargo test --locked --manifest-path src/kittxprobe/Cargo.toml --no-default-features --features ui-test
cargo clippy --locked --manifest-path src/kittxprobe/Cargo.toml --all-targets --features ui-test -- -D warnings
sh -n build_probe.sh
python3 -B -m unittest discover -s tests -p test_build_probe.py
```

Normal tests use temporary SQLite files and a local scripted TCP ELM327 emulator,
without car hardware, external services or remote networking.

| API / behavior | Tests |
|---|---|
| `ProbeCapture::{start,snapshot,request_stop,is_finished,reap,can_save,saved_to,save,reader}`, `CaptureReader::{read,read_view}`; producer identity; active-save rejection; checked PID/DTC requests only; gated automatic/explicit-view reads, finalization and ViewerUI compatibility | `tests/capture_tcp.rs`; `capture_tests.rs` |
| Save validation, standalone backup including WAL, timestamps/cutoff, original-file preservation, no overwrite, active-file rejection, SQLite-full failure with intact source and retry, cancelled/failed destination retry | `capture_tests.rs`; `ui_tests.rs` |
| `LiveSnapshot::{read,read_view}`, `bounded_view`, `LiveSnapshot::{full,view}`, `UnitTrack`, `SignalPlot`, `DtcRead`; all 560 decoded fields/27 units, ECU separation, read-only snapshots, raw/status-only handling, rolling/full/custom intervals, zoom detail reloaded from original samples, navigation limits, ten-hour bounded plots, partial ECU timestamps, released WAL readers, session-wide DTC history, latest-200 bound | `tests/live_snapshots.rs` |
| Transport modes, checked PID identity, validation, locking, Start without dialog, live plots, selected DTCs, Stop, save cancellation/failure/retry, restarting without saving while retaining the previous file, exit finalization, plot minimum height/scrolling, narrow layout and both themes | `ui_tests.rs` (`ui-test`) |
| Zoom buttons, pointer-anchored wheel zoom, drag/button panning, shared time view across separate unit groups, zoom versus track scrolling, reset before/after Stop, fixed view during capture, latest view wins over in-flight reads, disabled controls without data | `ui_tests.rs`: plot_buttons_wheel_drag_and_reset_share_one_view_across_unit_groups; live_start_stop_selected_dtcs_save_cancel_failure_and_retry |
| Editable serial port/baud options and macOS callout name handling | reused `../kittxrecorder/serial_ports_tests.rs` |
| Disconnect barrier waits for existing read leases, rejects new live reads, then allows fresh finalized snapshots | `capture_tests.rs`: finalization_barrier_waits_for_existing_readers_and_rejects_new_live_reads; `tests/capture_tcp.rs` |
| Release builder host/OS/CPU/ABI selection, native launch order, foreign/build-only behavior, probe-manifest clean without build/launch/host detection, rejected clean combinations, spaces, errors and exit statuses | root `tests/test_build_probe.py` |

Local native validation is on macOS ARM64. The desktop CI matrix includes macOS,
Windows and Linux; its remote results and real-car/serial hardware validation are
not claimed as local passes. The desktop UI is not an Android target.
