# KittxRecorder

A native GPUI Kit desktop trip recorder. It saves the existing version-1 SQLite
`.kittx` format through `kittx::datastore::Recorder`; the files open directly in
[ViewerUI](../viewerui/README.md). The parent library and existing binary are unchanged.

## Run and build

Use [KittxLauncher](../kittxlauncher/README.md) to open this UI inside its process.
This package retains its standalone binary and exposes `ui::open`, `ui::close`,
and `ui::run` behind the `ui` feature.
Only one Kittx UI may run per user, including direct launches. Closing Recorder
finalizes any active recording before returning to the launcher.

From the repository root:

```sh
cargo run --locked --manifest-path src/kittxrecorder/Cargo.toml
cargo run --locked --release --manifest-path src/kittxrecorder/Cargo.toml
cargo build --locked --release --manifest-path src/kittxrecorder/Cargo.toml
```

To build and run a release executable using the shell script:

```sh
./build_recorder.sh           # Build and run for the active compiler's host target
./build_recorder.sh windows   # Windows MSVC, retaining the host CPU architecture
./build_recorder.sh linux     # Linux GNU, retaining the host CPU architecture
./build_recorder.sh macos     # macOS, retaining the host CPU architecture
./build_recorder.sh --build-only # Build without launching
./build_recorder.sh clean        # Remove Cargo build artifacts
```

[build_recorder.sh](../../build_recorder.sh) works from any working directory.
After a successful build, it launches KittxRecorder when the target exactly matches
the compiler's host target. Foreign targets are built without launching, including
different libc or ABI targets on the same OS. `--build-only` skips launch and can
appear before or after the OS argument. Build failures prevent launch, and the
script returns the application's exit status when it runs.
Use Git Bash or another POSIX shell on Windows. Explicit OS selection supports
`x86_64` and `aarch64`; no argument preserves the exact host target, including its
Linux libc. Selecting a different OS requires its Rust target, native libraries,
linker, and SDK to be installed; the script does not install cross-compilation tools.

Use `clean` by itself to remove the recorder's Cargo build artifacts without
building or launching. It respects Cargo's configured target directory; recordings
outside that directory are untouched.

Script builds put the executable at
`src/kittxrecorder/target/<target-triple>/release/KittxRecorder`
(`KittxRecorder.exe` on Windows). For example, an Apple Silicon host defaults to
`src/kittxrecorder/target/aarch64-apple-darwin/release/KittxRecorder`.
The direct Cargo commands above, without `--target`, use
`src/kittxrecorder/target/release/KittxRecorder`. These are the default paths;
`CARGO_TARGET_DIR` or Cargo configuration can override the target directory.

This is an isolated Cargo package with its own
lockfile and Rust 1.92 minimum; normal bridge-library builds remain at Rust 1.85
and do not compile GPUI. The UI uses the same pinned GPUI Kit 0.7.3 as ViewerUI.
See the [viewer platform requirements](../viewerui/README.md#build-and-run) for
macOS, Windows MSVC, and Linux native/GPU dependencies. Mobile is not a UI target.

Default features enable both the UI and serial transport. A TCP-only desktop build:

```sh
cargo run --locked --manifest-path src/kittxrecorder/Cargo.toml --no-default-features --features ui
```

## Record a trip

The UI uses a 14 px base font and rounded buttons with highlighted outlines in
both light and dark themes, matching ViewerUI.
The dark theme shares KittxProbe's muted navy backgrounds, pastel blue accents,
and cool white text across panels, menus and controls.

1. Start your TCP emulator/adapter or attach your serial adapter. Choose **TCP/IP**
   (default `127.0.0.1:36000`) or **Serial** and enter its connection parameters.
   The serial port combobox lists detected ports and supports searching or typing
   a custom device path/COM name. Select the typed entry or press Down then Enter
   to confirm it. **Refresh ports** rescans attached adapters while preserving the
   selected port, including custom or disconnected devices. Entering Serial mode
   also refreshes the list. Scanning does not open or configure any port; the
   combobox and refresh control are locked while choosing a file or recording.
   Empty or unavailable discovery still permits manual entry. On macOS, detected
   `/dev/cu.*` callout paths are listed instead of matching `/dev/tty.*` aliases.
   Serial uses the supplied device/COM path and baud rate, 8N1, and no flow control.
   The editable baud-rate combobox offers common rates from 110 to 4000000 in
   numeric order, including 9600, 38400 (the default), 57600, and 115200. Search
   or type a custom positive integer rate and confirm the entry, as with the port
   combobox. Invalid, zero, or empty rates are rejected before recording starts.
   The rate is preserved when switching connection modes and is locked while
   choosing a file or recording. The adapter and driver determine which rates
   actually work.
   On macOS, the recorder uses the matching `/dev/cu.*` callout device when you
   enter a `/dev/tty.*` device and that callout exists. The log records both paths.
   Dial-in devices can block or report busy without carrier. You can also enter
   the callout directly (for example `/dev/cu.usbserial-1130`).
2. Choose **Start / stop** for an open-ended trip or **Fixed time** and enter the
   capture duration in whole seconds. Select measurement and DTC scan periods and
   the per-operation timeout. Intervals must be positive.
3. Use the searchable measurement combobox to check or uncheck measurements.
   The dropdown stays open as you toggle entries; press Escape or click outside
   to close it. Search filters labels/PID identifiers and preserves checked items
   outside the filtered list. The initial selection is all eight decoded
   measurements. **Select decoded**, **Select all PIDs**, and **Clear signals**
   update the same checked selection, including when a search is active.
   Only checked measurements are included in the recording configuration at
   Start; unchecked measurements are not sampled. DTC categories use independent
   checkboxes, initially stored/pending/permanent. Raw/undecoded PIDs remain
   available in the measurement dropdown with explicit raw labels.
4. Click **Start recording…**. The native save dialog appears before any connection
   is opened. Choose a new filename. Its extension is normalized to `.kittx` and
   existing files are rejected; the application never overwrites a recording.
   Cancel creates no session and opens no adapter connection.
5. The recorder connects, identifies the adapter, establishes VIN, discovers selected
   PID capability information, and performs selected beginning DTC scans. A valid
   and consistent VIN is required by the existing storage library. Unknown vehicle
   details stay unknown; the optional trip name has caller provenance.
6. Watch elapsed wall time, VIN, output filename, acquired/committed counts, and failed
   acquisitions in the footer. Settings, the measurement combobox, bulk selection
   buttons, and DTC checkboxes are locked while a save dialog or recording is
   active. Theme switching remains available during recording.
7. Click **Stop & save**, or allow the fixed-time capture to expire. The recorder
   performs selected final DTC scans, commits, disconnects, and finalizes the file.
   **Exit**, the native window close button, and the Quit action request Stop and
   wait for the worker before closing if recording is active.

Keyboard/menu commands: **Ctrl/Cmd+R** starts; **Ctrl/Cmd+Shift+R** stops;
**Ctrl/Cmd+Q** quits. The File menu exposes the same commands. Dark is the startup
appearance; the **Light theme / Dark theme** button switches the whole UI.

A DTC-only session is valid with no measurements selected; a measurement-only session
is valid with all DTC categories disabled. At least one signal or diagnostic category
must be selected. Use a new save file for every subsequent trip.

The settings panel groups **Connection**, **Recording mode**, and **Vehicle type**
into independently expandable accordions. Connection and recording mode start open;
vehicle type starts closed. Collapsing a section preserves its settings.

In **Vehicle type**, the **Car brand** and **Car model** comboboxes provide searchable
offline suggestions. Choose a brand to enable its model selector. Changing or clearing
the brand clears the previous model. For an unlisted brand or model, type its name
in the dropdown search and select the matching new row (or use the arrow keys and
Enter). These suggestions are a starter list, not a complete catalog. Searching without confirming
does not change the selection. Both fields are optional and lock with other settings
during the save dialog or recording. Confirmed choices are saved as `make` and `model`
with `caller` provenance in the existing `.kittx` vehicle metadata; ViewerUI displays
them in its Vehicle accordion. They are independent of the ECU-read VIN.

## Measurements and DTC coverage

Decoded, numeric measurements:

| Measurement | Source / unit |
|---|---|
| Engine load | Mode 01 PID 04, percent |
| Coolant temperature | Mode 01 PID 05, °C |
| Manifold pressure | Mode 01 PID 0B, kPa |
| Engine RPM | Mode 01 PID 0C, rpm |
| Vehicle speed | Mode 01 PID 0D, km/h |
| Intake temperature | Mode 01 PID 0F, °C |
| Throttle position | Mode 01 PID 11, percent |
| Adapter battery voltage | ATRV, adapter supply volts (not ECU PID 42) |

The measurement combobox names the documented current-data PIDs, including fuel
trims, oxygen sensors, fuel/rail/EVAP pressures, ECU voltage, oil/catalyst/exhaust
temperatures, torque, turbo/DPF/SCR/NOx data, fuel rates, odometer, transmission,
hybrid/EV battery packs, electric motors, and fuel-cell data. Each sensor-family
member has its own PID entry. Names are searchable, and each raw entry displays
its hexadecimal **Mode 01 PID XX** so it can be matched to the requested data.
The display-name catalog is in [pid_catalog.rs](pid_catalog.rs), based on
[DashLogic](https://www.dashlogic.com/docs/technical/obdii_pids),
[CSS Electronics](https://www.csselectronics.com/pages/obd2-pid-table-on-board-diagnostics-j1979),
and the [OBDb decoder definitions](https://github.com/OBDb/SAEJ1979/blob/main/signalsets/v3/default.json).

Additional named PIDs are **raw / undecoded**, rather than numeric/plot-ready.
Selecting one creates `Signal::RawPid(0xXX)`, sends `01XX`, and saves the complete
positive ECU payload, including service/PID bytes and reassembled multi-frame data.
Separate responding ECUs retain separate identities and measurements. The label
never determines request bytes. Unknown slots remain explicitly **unassigned /
unverified** raw choices; the app does not claim those slots are standard data.
There are 248 total selectable measurement items: 247 PIDs and adapter voltage.
Supported-PID bitmap queries at 00/20/40/60/80/A0/C0/E0 are handled by discovery.
PID 02 (the freeze-frame DTC) belongs to Service 02 and is excluded from this
live Service 01 picker. Freeze-frame capture is not added by this catalog update.
Known decoded PIDs appear only once. Raw captures retain application bytes,
ECU identity, timestamps, and evidence in the same schema. ViewerUI can inspect raw
samples, but cannot plot them as numeric values without an appropriate decoder.
No formulas or interpretations are invented for unknown or reserved PIDs.

The combobox is a request catalog, not a statement that a vehicle supports every
entry. **Select all PIDs** can cause many unsupported/failed reads and substantially
reduce effective sampling rates. Discovery and acquisition outcomes are preserved.
Nominal periods do not guarantee sample frequency: requests run sequentially and
actual completion timestamps are recorded.

Stored (03), pending (07), and permanent (0A) DTCs are independently selectable. They
include manufacturer-specific code numbers returned by standard OBD services;
unknown code descriptions are not guessed. Selected categories are read at the
beginning, periodically, and at the end. Read times do not establish fault onset.
Failed/unsupported scans remain failures, never successful empty results.

For module-specific diagnostics, enable **Manufacturer-specific diagnostic read
profile (raw)** and fill in the left-panel fields:

- Profile ID, version, and namespace to identify the documented request.
- Hexadecimal request bytes from the module documentation (not an AT command).
- Optional explicit header, receive filter, and target when physical addressing
  is required. Header widths: 3 digits for 11-bit CAN, 8 for extended CAN, 6 for a
  legacy header. Targets use 2 legacy, 3 standard CAN, or 8 extended CAN digits.

The profile runs with the selected DTC scan schedule and preserves unparsed replies;
it does not claim decoded manufacturer DTC counts. Only supply documented **read**
requests. Standard reads do not clear DTCs; the library rejects OBD clearing service
04 as a recording profile. No reset, reconnection, DTC clearing, or file repair is
performed as implicit recovery.

## Diagnostics and troubleshooting

Every native recording job writes a diagnostic sidecar next to the chosen file:
`trip.kittx.log`. The footer shows its path and the most recent bridge/ECU error,
including individual ECU failures when another ECU replied successfully. Diagnostics
also appear on standard error when launched from a terminal. Long footer messages
wrap and scroll vertically without displacing the recording controls.

The default info level records connection parameters (including serial baud),
initialization/adapter identity, VIN/support/DTC read stages, failures, and final
acquisition counts. A failed read includes its request, protocol, elapsed time,
timeout/recovery details, partial response, and the most recent 32 transport events.
Bytes are shown as escaped ASCII and hexadecimal, so `\r`, `\n`, NULLs, and prompt
boundaries are visible. Short timeouts during initial synchronization probes are
expected; the terminal operation result determines whether connection failed.

For every transmitted/received chunk and successful decoded read, enable debug
logging before starting the app (macOS/Linux or Git Bash):

```sh
KITTX_RECORDER_LOG=debug ./build_recorder.sh
```

In PowerShell, set `$env:KITTX_RECORDER_LOG = 'debug'` before launching the binary.
Unset the variable or set it to `info` for the default level. Each emitted line is
flushed immediately. Existing diagnostic sidecars are appended to; `.kittx` files
are still never overwritten. Logging write failures fall back to the console and
do not change transport results. Debug logs grow with trip duration; the normal
info mode retains only bounded recent wire context in memory until an error.
Individual byte dumps are capped at 4096 bytes and label truncation; the recording
retains raw evidence under its existing storage limits. Logs can contain VIN and
vehicle responses, just like the recording.

If recording fails, inspect the first `ERROR` and preceding `CONTEXT`/`DEBUG` lines.
Connection errors identify the host/port or serial path and baud; initialization
errors include the failed step and nested cause. A valid ECU VIN is still required
to start a session. `NO DATA`, an ECU negative reply, and a host timeout remain
distinct failures. No usable VIN produces a `start_failed` recording with raw
evidence rather than inventing vehicle identity.

With the emulator already running at `127.0.0.1:36000`, this opt-in smoke test runs
the real recording worker without the GUI and checks the resulting file in ViewerUI:

```sh
cargo test --locked --manifest-path src/kittxrecorder/Cargo.toml --no-default-features --test worker_tcp running_emulator_records_selected_signals_and_dtcs_with_diagnostics -- --ignored --nocapture
```

It records all eight decoded signals and all three standard DTC categories for two
seconds, checks the emulator VIN and zero failed acquisitions, and prints a temporary
directory containing the `.kittx` file and log for inspection. Ordinary automated
tests use local scripted peers and do not depend on this emulator.

## Timing, durability, and shutdown

One application-owned worker owns the synchronous client and recorder. UI progress
uses one latest-value slot and a 100 ms UI refresh; it does not queue samples in RAM.
Each acquisition is committed by the existing SQLite writer. Transport framing,
raw evidence, per-ECU identity, schema/version, and timestamps remain unchanged.
The UI does not issue overlapping commands or add a bridge-library worker.

Stop is cooperative: an in-flight operation and synchronous preparation can finish
before the run loop sees it. Final DTC scans and finalization take additional time.
The fixed duration is the library's capture budget; preparation/finalization may make
wall-clock runtime longer. Elapsed UI time includes preparation and finalization.
DNS and OS serial-open behavior have the existing library's timeout limitations.

The normal close path waits for the worker to finish. A forced process/OS termination
can still leave an unfinished file; committed acquisitions remain in SQLite. Keep
WAL/SHM sidecars with an unfinished file. The UI does not automatically recover,
resume, migrate, or overwrite such files. Failures show their reason and output path;
the file remains available for read-only inspection or explicit later recovery.
See [README_DATASTORED.md](../../README_DATASTORED.md) for storage semantics.

A completed session can contain failed acquisitions or incomplete final DTC coverage.
Inspect the failure count and coverage message. A failure to establish VIN produces
a `start_failed` file rather than a fabricated VIN or successful trip.

## Verification

```sh
sh -n build_recorder.sh
python3 -B -m unittest discover -s tests -p test_build_recorder.py
cargo fmt --manifest-path src/kittxrecorder/Cargo.toml --check
cargo test --locked --manifest-path src/kittxrecorder/Cargo.toml --features ui-test
cargo test --locked --manifest-path src/kittxrecorder/Cargo.toml --no-default-features
cargo test --locked --manifest-path src/kittxrecorder/Cargo.toml --no-default-features --features ui-test
cargo clippy --locked --manifest-path src/kittxrecorder/Cargo.toml --all-targets --features ui-test -- -D warnings
```

Tests use a fake clock/data source and a local loopback TCP ELM327 transcript server;
no ECU, serial hardware, or external network is required. ViewerUI's actual document,
plot-frame, and raw-sample readers verify recorded files.
On macOS/Linux, the shell-script tests use fake compiler/Cargo tools to verify OS
selection, quoted paths, working-directory independence, native launch after a
successful build, build-only mode, skipped foreign launches, CRLF compiler output,
and build/application failure propagation. They also verify standalone cleanup,
cleanup failure propagation, and rejection of combined clean/build options.
They do not build or launch the GUI.

| API / behavior | Coverage |
|---|---|
| `CaptureItem`, `capture_catalog`, `CaptureSettings::{default,config}` | catalog_paths_intervals_and_validation; timed_capture_is_readable_and_plottable_by_viewer |
| Named PID coverage, sensor families, label-to-request identity, raw fallbacks, exclusion of Service 02/bitmap entries | tests/pid_catalog.rs: named_catalog_covers_documented_live_pids_without_changing_request_identity |
| Every raw picker selection through real OBD requests and SQLite, newer battery/motor PIDs, multi-frame payload completeness, separate ECU replies | tests/worker_pid_catalog.rs: every_raw_catalog_selection_sends_its_own_pid_and_preserves_complete_ecu_payloads |
| `recording_path`, `positive_duration`, `ProfileAddress`, `profile_address` | catalog_paths_intervals_and_validation; manufacturer_profile_addressing_and_destructive_service_rejection |
| `SessionControl::{new,request_stop,snapshot}`, `LiveStatus`, all `Phase` variants | manual_stop_finalizes_and_keeps_selected_measurements_only; failed_start_retains_compatible_file_and_reports_failure; profile-addressing test; TCP worker test |
| `run_session`, timed/manual scheduling, all DTC kinds, raw profiles, failures, VIN, signal-only/DTC-only | tests/recordings.rs |
| `RecordingJob::{spawn,request_stop,snapshot,is_finished,reap}`, drop/worker failure | worker_failures_are_terminal_and_reap_is_nonblocking; tests/worker_tcp.rs |
| `RecordingJob::spawn_with_disconnect_hook`: worker-thread barrier before disconnect/finalization for applications reading active files | `../kittxprobe/tests/capture_tcp.rs`; `../kittxprobe/capture_tests.rs`: finalization_barrier_waits_for_existing_readers_and_rejects_new_live_reads |
| GPUI mode/checklist/theme controls, independent settings accordions, linked vehicle comboboxes/custom values, optional metadata, narrow layout, locked settings, save cancellation, validation, normalized extension, close while active | ui_tests.rs (ui-test feature) |
| `VehicleOptions::{brands,models}`, search, confirmed custom values and single selection | accordions_and_vehicle_selection_metadata |
| Detected serial ports, sorted/deduplicated names, macOS callout preference, custom/disconnected entries, selection/cancel/clear, refresh, mode switching, locked combobox | serial_ports_tests.rs; ui_tests.rs: serial_combobox_selects_detected_or_custom_ports_and_preserves_selection |
| Baud-rate presets in numeric order, 38400 default, custom values, cancelled searches, invalid/cleared values, mode switching, locked combobox | serial_ports_tests.rs; ui_tests.rs: baud_combobox_defaults_presets_custom_validation_and_locking |
| Measurement multi-select combobox, filtered check/uncheck by PID identity, raw choices, default/bulk selection, exact capture configuration, narrow layout, locked controls, save cancellation | ui_tests.rs: measurement_combobox_toggles_by_identity_and_records_only_checked_items; modes_checklist_themes_save_cancel_and_validation |
| Unchecked measurement PIDs are not requested or stored | tests/worker_tcp.rs: real_worker_tcp_timed_and_manual_recordings_open_in_viewer; tests/recordings.rs: manual_stop_finalizes_and_keeps_selected_measurements_only |
| Caller-provided vehicle make/model and provenance remain readable in ViewerUI | timed_capture_is_readable_and_plottable_by_viewer |
| `LiveStatus::{diagnostics_path,last_error}`, flushed logs, recent wire context, byte escaping, nested causes, partial writes, per-ECU errors, log I/O failures | diagnostics_tests.rs; worker_tcp.rs |

Desktop CI builds/tests macOS, Windows, and Linux. Local native validation is on
macOS ARM64 with Rust 1.98.1; CI configuration does not imply local Windows/Linux
runtime validation. Serial hardware, a real ECU, vendor decoding, and platform
visual/accessibility acceptance are not claimed by these fixtures.
