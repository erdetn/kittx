# .kittx recording viewer

Native, read-only desktop viewer built with GPUI Kit 0.7.3. It is an isolated
Cargo package: normal `kittx` library builds do not compile GUI dependencies or
change the existing application's binary target.

## Build and run

Use [KittxLauncher](../kittxlauncher/README.md) to open this UI inside its process.
This package retains its standalone binary and exposes `ui::open`, `ui::close`,
and `ui::run` behind the `ui` feature.
Only one Kittx UI may run per user, including direct launches. Close this window
to return to the launcher when started from it.

From the repository root:

```sh
./build_viewerui.sh              # Release build for the host, then run
./build_viewerui.sh windows      # Windows MSVC, retaining the host CPU
./build_viewerui.sh linux        # Linux GNU, retaining the host CPU
./build_viewerui.sh macos        # macOS, retaining the host CPU
./build_viewerui.sh --build-only  # Build for the host without launching
./build_viewerui.sh clean        # Remove Cargo build artifacts
```

The [shell script](../../build_viewerui.sh) works from any working directory and
requires a POSIX shell (for example, Git Bash on Windows). Without an OS argument
it preserves the active Rust compiler's exact host target, including Linux libc
and Windows ABI. Explicit OS selection supports `x86_64` and `aarch64`.
Native builds run after a successful build; targets different from the compiler's
host target are built without launching. `--build-only` can appear before or
after the OS argument. Both build and run use `--locked --release`, select only
`kittx-viewer`, respect Cargo's target-directory configuration, and return the
build/application exit status.

Use `clean` by itself to remove the viewer's Cargo build artifacts without
building or launching. It respects Cargo's configured target directory; recordings
outside that directory are untouched.

The existing Python script provides a build-only alternative:

```sh
python3 build_viewer.py          # Release build for the host OS/architecture
python3 build_viewer.py windows  # Release build for Windows (MSVC)
python3 build_viewer.py linux    # Release build for Linux (GNU/glibc)
python3 build_viewer.py macos    # Release build for macOS
```

The script requires Python 3.9+ (`py -3 build_viewer.py` on Windows) and can be
invoked by its absolute path from another directory. It always uses `--release`
and `--locked`, builds only `kittx-viewer`, and returns Cargo's failure exit code.
An explicit OS retains the active Rust compiler's architecture (`x86_64` or
`aarch64`); omitting it preserves the exact host target, including its Linux libc.
The default output is `src/viewerui/target/<target-triple>/release/kittx-viewer`
(`.exe` for Windows). Cargo target-directory configuration remains respected.

Selecting another OS selects its Rust target; it does not install the Rust target,
native dependencies, cross-linker, or OS SDK. Those must already be configured.
Windows uses MSVC, Linux uses GNU/glibc, and macOS requires Apple's SDK/toolchain.
Building on the destination OS is the straightforward option when a cross-build
toolchain is unavailable. See the platform setup link below.

For a development build and immediate launch:

```sh
cargo run --locked --manifest-path src/viewerui/Cargo.toml
cargo run --locked --manifest-path src/viewerui/Cargo.toml -- trip.kittx
```

Use `--release` for optimized interactive performance. The first build downloads
and compiles the framework. The executable is
`src/viewerui/target/{debug,release}/kittx-viewer` (`.exe` on Windows).

The viewer manifest requires Rust 1.92 or later; the parent library remains at
Rust 1.85. The viewer has its own lockfile and workspace boundary. GPUI Kit is
Apache-2.0; bundled assets retain their respective upstream licenses.

[GPUI Kit's installation guide](https://gpui-kit.com/docs/installation/)
specifies macOS 15+, Windows 10+ with MSVC/C++ build tools and CMake, and Linux
with native dependencies and a graphical Wayland/X11 session with a Vulkan driver.
Use that guide for setup; `.github/workflows/viewer.yml` lists Linux build packages.
A headless server can run model/reader tests but cannot display the application.

## Interface

- The base font is 14 px, one step below the framework's 16 px default, in both
  themes. Buttons use rounded corners and highlighted outlines; plot legend
  buttons retain their series colors.
- The dark theme shares KittxProbe's muted navy backgrounds, pastel blue accents,
  and cool white text across panels, menus and controls.
- **Light theme / Dark theme** in the toolbar switches the entire viewer between
  white and dark appearances. The viewer starts in dark mode. The button names
  the theme it will switch to.
- **File → Open .kittx…**, or **Ctrl/Cmd+O**, opens a recording. An optional command
  line path opens it on startup. The native picker accepts any extension so renamed
  recordings can be validated by their schema. Cancel leaves the current file open;
  invalid or unsupported files report an error and retain the previous document.
- **File → Exit**, the Exit button, or **Ctrl/Cmd+Q** closes the viewer and quits.
  Closing the native window also quits the application when no windows remain.
- The left sidebar displays session status, VIN and provenance, vehicle fields,
  UTC timestamps in integer microseconds, observed duration, committed acquisition
  cutoff, producer, connection segments, ECU identities, sample counts, sampling
  policy, and diagnostic coverage. GPUI Kit accordion sections group **Session**,
  **Vehicle**, **Diagnostics**, **ECUs & connections**, and **Sampling**. Session and
  Vehicle start expanded; sections toggle independently and keep their state while
  navigating or changing themes. Optional metadata displays its plain value, or `-`
  when absent. Sidebar labels use muted text; values use the theme's foreground
  color and semibold weight to stand out in both light and dark modes.
  **Info** collapses the entire sidebar. Drag panel separators
  to resize the sidebar, signal selector, and plotting area. Metadata and signal
  labels wrap within their panels; scrollbars reveal long sections. Compact legend
  and history controls stay within panel width, with full history labels in tooltips.
  The toolbar wraps when the window is narrow.
- Choose recorded PIDs in the searchable multi-select combobox. Search by name,
  stored identifier/PID, unit, or ECU, then check/uncheck entries without closing
  the dropdown. Searches preserve checked entries outside the results. The
  **Select all** and **Unselect all** buttons directly below the combobox apply to
  all numeric series, across searches and ECUs. The ECU filter controls which
  checked series are plotted; legend buttons also uncheck their entries.
  Speed and RPM are selected initially when recorded; otherwise the
  first numeric series is selected. Raw Service 01 recordings are also interpreted
  using the offline [numeric PID catalog](pids/README.md): 560 measurements/counters
  across all 195 measurement-bearing PIDs in [PIDS.MD](../../PIDS.MD). Multi-value
  PID blocks expand into separate named series with physical units (for example,
  individual oxygen sensors, battery packs, and exhaust-temperature sensors).
  Original raw rows appear as disabled **raw only** entries in the dropdown.
  Status/enumeration-only PIDs and unverified/vendor signals remain raw and cannot
  be plotted as analog values. Original evidence remains in the recording; there
  are no per-signal raw-data buttons.
- Different units use vertically stacked tracks with one shared time axis.
  Same-unit series overlay; every ECU/segment remains a separate series. Legend
  entries identify unit and sender. Y limits automatically fit finite values and
  pad constant values. Curves are straight segments; isolated points remain visible.
- Mouse wheel zooms around the pointer. **+/−** zoom around the center. Drag in
  **Pan** mode to translate time; click **Pan** to switch to **Box zoom**, then drag
  a time interval. **Fit / Auto Y** restores the observed range and auto scaling.
  Keyboard: **+/−**, **left/right**, **F** to fit. **Escape** cancels a drag or pinned
  inspection. Pointer navigation can extend beyond the recorded interval.
- Hover the plot for the nearest original sample, including value, unit, sender,
  UTC, sample ID and acquisition window. Click to pin it; Escape resumes hover.
  A shared cursor guide crosses the tracks and DTC lane. **Hide/Show details**
  controls the inspection area.
- DTC observations occupy the bottom lane. Their timestamps are **read completion
  times**, not fault onset times. DTC visibility, categories and ECU filtering are
  independent of numeric selections. Positive observations and uninterpreted raw
  diagnostics are visible initially. Enable **Include empty / failed scans** for
  status observations. Hover a DTC point to highlight it and preview its details.
  Click to keep it highlighted and pin its details in the panel below the plot;
  the panel opens automatically and scrolls to the beginning. It shows recorded
  codes, elapsed observation time from recording start, UTC microseconds, category,
  ECU, outcome and available original evidence. Clicking another point replaces
  the selection; Escape clears it and resumes hover. Point outlines follow the
  light/dark theme. The scan history also exposes empty/failed results.
- **DTC mode** cycles all observations, first-observed codes, and successful code-set
  changes. These projections use full history before the viewport. Failed scans
  never imply an empty set, disappearance does not prove repair, and manufacturer
  bytes remain uninterpreted. Nearby reads of the same kind share a point;
  selection preserves each member's ECU/category and expands up to 100 unique
  scans, with a notice to zoom in when more remain. History rows select their
  exact read/group, including reads at the same timestamp.

## File and resource semantics

The viewer opens SQLite with read-only flags, validates the application/schema IDs,
and acquires its overview in a short consistent transaction. All later measurement
and scan queries use the acquired committed sequence. A file still being recorded
therefore stays a static view until reopened. It does not hold a long-lived read
transaction that blocks WAL checkpoints. A replaced session is rejected on reload.

Keep SQLite WAL sidecars with unfinished files. No immutable-mode shortcut ignores
the WAL; no recovery, repair, migrations, finalization, DTC clearing, or ECU access
occurs. A recording/interrupted/failed status remains exactly as recorded. Unknown
VINs, fields, and diagnostic coverage stay unknown. Main-file bytes are checked
before/after viewing in the integration fixtures.

Elapsed storage and viewport coordinates use integer microseconds; drawing uses
relative floating-point coordinates. Display offsets use the session start minus
creation UTC. Schema v1 has no explicit monotonic session-start field, so a wall
clock adjustment during preparation can affect that display offset; ordering and
plot spacing still use recorded monotonic elapsed times. Exact UTC remains visible.

Each series query retains original first/last/minimum/maximum samples per pixel
bucket and viewport neighbors. Continuity is computed **before** downsampling:
failed reads, partial samples, and gaps greater than three times the applicable
sampling period break a line. Selection-change history supplies the period; absent
policy uses a one-second fallback. Different connection segments never share a
series. Sample inspection queries original rows rather than the plotted envelope.
For raw PID projections, unsupported sensor bits, no-result encodings and missing
field bytes also break lines. Complete fields preceding a truncated tail can still
be shown. Decoded values retain original ECU identity, sample IDs, operation times,
and the document's frozen acquisition cutoff; no decoded cache or migration is
written into the recording. A series is listed only when a valid value exists.

There is one active viewport job plus a coalesced latest request, one active open
job plus its latest pending path, and one active inspection plus its latest pending
position. Generations reject stale file results; viewport cancellation is checked
between queries. An already executing SQLite query completes before cancellation.
The bridge library retains its synchronous caller-owned scheduling contract.

The current viewport is the plot cache; historical viewports are recomputed. At
most 4096 distinct selected series and approximately 96,000 original plot points
plus viewport neighbors are loaded, allowing all numeric fields to be selected.
DTC clustering returns at most 20,000 groups, otherwise asks for a narrower view.
The history shows the first 200 groups; each scan displays at most 1000 code entries,
and cluster details show at most 100 scans per
inspection with an explicit zoom-in notice. Raw evidence displays the first 4096
bytes; the source file retains all stored bytes. This is bounded rendering, not a
claim of a measured 64 MiB whole-process memory limit. No universal frame-rate or
multi-million-row latency guarantee has been measured.

## Validation and coverage

```sh
sh -n build_viewerui.sh
python3 -B -m unittest discover -s tests -p test_build_viewerui.py
python3 -B -m unittest discover -s tests -p test_build_viewer.py
cargo fmt --manifest-path src/viewerui/Cargo.toml --check
cargo test --locked --manifest-path src/viewerui/Cargo.toml
cargo test --locked --manifest-path src/viewerui/Cargo.toml --features ui-test
cargo test --locked --manifest-path src/viewerui/Cargo.toml --no-default-features
cargo clippy --locked --manifest-path src/viewerui/Cargo.toml --all-targets -- -D warnings
cargo test --all-features
```

| API / behavior | Tests |
|---|---|
| Shell release build/launch order, exact host and OS/CPU selection, build-only/foreign-target behavior, standalone clean command and failure propagation, rejected clean combinations, working-directory independence, spaces in paths, CRLF compiler output, invalid arguments, failures and program exit statuses | root `tests/test_build_viewerui.py` (fake tools on POSIX hosts; no actual cross-compilation or GUI launch) |
| `Viewport::{new,time,fraction,zoom,pan}`, finite padded `range` | `model.rs` unit tests |
| `signal_label`, original continuity `gap`, all `MarkerKind` variants and `marker_kind` | `tests/recordings.rs`: defaults_labels_and_gap_identity; marker_semantics_are_distinct |
| `PlotDtcMarker`, `plot_dtc_markers`, `hit_dtc_marker`: pixel groups, exact hit targets, all kinds, edges and invalid dimensions | `tests/dtc_plot.rs` |
| `Document::{open,full,frame,inspect,scan_details,cluster_details}`, `Frame` | document_open_plot_inspect_and_cluster; missing_wrong_version_and_empty_recordings |
| `frame_with_cancel`, invalid selections, file replacement | cancellation_and_invalid_series_are_results |
| `raw_sample`, `Document` cutoff after file growth | raw_series_bytes_and_frozen_document |
| `pid::{Field,fields,field}`, `Field::{decode,signal,source_signal}`, complete PIDS.MD coverage, signed/scaled values, support/no-result flags and truncated fields | `tests/pid_decoding.rs` |
| Raw-PID overview expansion, all 560 numeric fields, units, ECU separation, sample counts, envelopes, nearest inspection, failure/period/invalid-value gaps, frozen cutoff, original bytes and read-only files | `tests/pid_recordings.rs` |
| PID combobox check/uncheck, search persistence, stable ECU/series identity, raw-only disabled entries, numeric plots, bulk/legend synchronization, replacement-file catalog and compact layout | `ui_tests.rs`: pid_combobox_toggles_by_identity_and_shares_bulk_and_legend_selection; native_layout_buttons_and_keyboard |
| `marker_details`, exact single reads, code/relative/UTC time, raw payload, nullable ECU identity, snapshot and 100-scan bounds | selected_dtc_groups_show_codes_times_and_original_raw_payload; dtc_cluster_selection_is_bounded_and_does_not_mix_unknown_ecu_with_known_ecus |
| `RecordingReader::{overview,plot_samples,nearest_sample}`, all new row structs | root `tests/datastore_view.rs`: overview_envelope_cutoff_nearest_and_validation; catalog_keeps_raw_units_and_senders_separate |
| `dtc_markers`, `dtc_markers_mode`, all `DtcDisplayMode` variants, `DtcMarker` | dtc_clustering_includes_empty_failed_raw_and_uses_completed_time; dtc_projections_use_history_outside_view_and_ignore_failed_scans |
| WAL, read-only, schema, six-hour bounded envelope, period history | wal_reader_releases_snapshot_and_never_updates_session; six_hour_recording_has_bounded_points_and_period_history; gaps_period_changes_and_failure_survive_decimation |

Local validation uses macOS 26.6.2 ARM64 and Rust 1.98.1: native compilation/linking,
application startup with `trip.kittx`, model/reader tests, formatting and Clippy.
The `ui-test` feature enables GPUI's test platform: layout, toolbar clicks, keyboard
Escape, picker cancellation, failed replacement retention, and light/dark rendering
are tested in `ui_tests.rs`. Exit-button and Quit-action regression tests also
verify that the viewer window is removed. It does not replace actual OS/GPU visual acceptance.
The DTC interaction test covers point hover, grouped stored/pending codes, pinned
selection changes, automatic details opening/scroll reset, exact history selection,
empty lane clicks, Escape, and selection retention when changing themes.
Windows/Linux builds and the declared Rust 1.92 floor have CI jobs, but their results
are not claimed as local passes. Accessibility, GPU frame-time measurements, and
full native visual acceptance remain manual checks.

The parent library's TCP-only Android target also compiles with the new reader
extensions using the local Android NDK. The desktop GUI is not an Android target.
