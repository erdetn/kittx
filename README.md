# kittx — experimental ELM327 library and desktop tools

kittx is an experimental Rust project for exploring synchronous communication with
an ELM327 adapter or a raw TCP emulator, vehicle-data recording, and desktop
inspection and analysis of recordings. It is intended for experimentation and
evaluation. APIs, desktop workflows, and the `.kittx` file format may change;
stability and production readiness are not guaranteed.

The package contains a library and a separate binary target; application scheduling
belongs in the binary/application layer. The desktop tools described below are
also experimental.

## ECU recording

Enable the optional `datastore` feature to record selected signals and timestamped
DTC scans into SQLite `.kittx` session files.

For a desktop capture app with live PID plots and selected DTC reads, run
[KittxProbe](src/kittxprobe/README.md) using `./build_probe.sh`. It supports serial
or TCP/IP, Start/Stop capture, and saving a `.kittx` recording after Stop.

## Basic use

```rust
use kittx::{Client, Initialization, Protocol, TransportConfig};
use kittx::obd::Pid;
use std::time::Duration;

let mut client = Client::with_initialization(
    TransportConfig::tcp("127.0.0.1", 35000),
    Initialization::obd(Protocol::Automatic),
);
client.connect(None, |result| result.unwrap());
client.read_voltage(None, |result| println!("{result:?}"));
client.read_pid(Pid::EngineRpm, Some(Duration::from_secs(10)), |result| {
    for measurement in result.unwrap() {
        println!("{:?}: {} rpm", measurement.sender, measurement.value);
    }
});
client.disconnect(None, |result| result.unwrap());
```

For desktop serial, use `TransportConfig::Serial(SerialConfig::new("/dev/ttyUSB0", 38400))` (a COM name such as `COM12` on Windows). Serial configuration exposes baud, data bits, parity, stop bits, and flow control. Opening/configuring the port uses `serial2`; default ELM settings are 8N1 with no flow control. Set the baud to match your adapter, not an assumed universal default.

On macOS, use the `/dev/cu.*` callout device. Standard baud rates are reapplied
through termios because some USB serial drivers accept serial2's `IOSSIOSPEED`
ioctl without changing the actual baud. Nonstandard rates retain serial2's ioctl
path. A USB adapter on `/dev/cu.usbserial-1130` was verified at 38400 baud with
initialization, identification, and supply-voltage reads. VIN and RPM returned
`NO DATA`; successful vehicle requests and recording remain unverified.

Constructors perform no I/O. `Client::new` connects without initialization. `with_initialization` stores steps for `connect` to execute. `with_options` configures deadlines/limits without initialization; `with_connector` accepts options, an initialization profile, and a `SystemConnector` or custom connector for full configuration.

All blocking operations invoke a completion callback inline on the invoking thread, once, including failures. There are no library worker threads, async runtime, polling interface, or command queue. Each operation exclusively borrows the client; invoke the next operation after the previous call returns. Callback panics propagate; normal completion bookkeeping occurs before the completion callback.

## Commands and responses

`Client::execute(Command, timeout, callback)` is the typed core/extended bridge interface. See the command module for fixed commands, toggles, byte settings, CAN IDs/headers, protocol selection, flow control, programmable parameters, calibration, and identifiers. Typed commands include range validation and explicit variants for aliases and wire widths.

```rust
# use kittx::{Client, TransportConfig, Command};
use kittx::command::{Toggle, ByteSetting};
# let mut client = Client::new(TransportConfig::tcp("127.0.0.1", 35000));
client.execute(Command::Toggle(Toggle::Echo, false), None, |r| {
    println!("{r:?}");
});
client.execute(Command::Byte(ByteSetting::ResponseTimeout, 0x32), None, |r| {
    println!("{r:?}");
});
```

`Response` contains a typed `ResponseValue` and a `Transaction` retaining lines, classified records, and raw bytes (including echo and any inserted NULLs). Settings ACKs, voltage, firmware identity, protocol numbers, bytes, buffer dumps, and PPS items are typed. KW/CS and protocol descriptions preserve text because the source does not establish a complete universal layout. `Response::parse` and `Decoder` are usable independently for generation/parsing without a live transport; specialized streaming, sleep, and baud framing still require their client paths.

With known E0, a bus reply equal to the request is preserved as data. When echo state is unknown, an exact normalized command-line match is treated as echo; initialize E0 for arbitrary protocols that may echo their payload at the application level. Raw bytes remain available either way.

`send_vehicle` sends hexadecimal payloads and returns the bridge transaction. An optional response count is a single nibble, not a transmitted payload byte. The default is no count limit. `Command::Repeat` repeats only a known completed ordinary bridge command; it is disabled after failed/interrupted writes, unknown startup state, or special operations. High-level OBD operations issue preparatory/query AT commands, so their final bridge command may be DPN rather than the vehicle request.

`Command::RawAt` is an ordinary-transaction escape hatch for emulator/vendor commands. It rejects embedded terminators, control characters, and known specialized baud/sleep/reset/monitor commands; use typed operations for those. Raw commands invalidate runtime setting assumptions. Caller-directed typed operations may change ECU/EEPROM state: initialization and recovery never silently clear DTCs or program identifiers/PPs.

## OBD operations

The initial application API includes `supported_pids`, `discover_pids`, `read_pid`, `read_vin`, `read_dtcs`, and `clear_dtcs`. Live PIDs cover engine load, coolant temperature, manifold pressure, engine RPM, vehicle speed, intake temperature, and throttle position. Units and sender identity accompany decoded measurements.

High-level OBD operations query DPN if the selected protocol is unknown, normalize H1/CAF1/D0/S1/R1 and disable extended addressing as needed, send the request, then query the active DPN and reassemble by sender. These steps share a total deadline and remain configured afterwards. Known J1939/user protocols are rejected before sending a high-level OBD request. Raw/extended APIs remain available for custom configurations and manufacturer requests.

Responses from several ECUs are collected through the prompt. CAN ISO-TP reassembly supports extended addresses, sequence wrap, length/padding checks, and interleaved senders. Legacy checksum/header parsing and VIN application segmentation are separate. H0 formatted multi-ECU ambiguity, missing/duplicate frames, and malformed payloads produce errors. DTC results combine legacy response lines by ECU. A final negative response from any ECU fails a high-level decode, preserving the complete request-response transcript in `Error.partial`, including other ECUs' replies; use `request_obd` when the application wants to handle individual negative replies itself.

`clear_dtcs` is a dedicated explicitly invoked service 04 operation. It erases stored diagnostic data and freeze frames, but not permanent trouble codes. The caller decides and confirms whether clearing is intended. No initialization or retry path invokes it.

J1939 bridge commands, MP/DM1 monitoring, and raw replies are available. J1939 SPN/FMI application decoding and arbitrary outgoing ISO-TP multiframe transmission are deferred.

## Monitoring

```rust
# use kittx::{Client, TransportConfig};
use kittx::{MonitorControl, command::Monitor};
use std::time::Duration;
# let mut client = Client::new(TransportConfig::tcp("127.0.0.1", 35000));
client.monitor(
    Monitor::All,
    Some(Duration::from_secs(30)),
    |record| {
        println!("{record:?}");
        MonitorControl::Stop
    },
    |result| println!("monitor ended: {result:?}"),
);
```

Event callbacks run repeatedly on the invoking thread. Return Stop to interrupt and consume output to the prompt; one completion callback reports the summary/error. MA, MR, MT, DM1, and both MP widths/count forms are supported. Annotated invalid frames are delivered as such and retained; terminal errors are not converted into valid frames. Spaced CAN header/data lines produce `Record::CanFrame`; compact/unknown presentations remain raw hex or unknown records and can be parsed with an explicit `obd::Layout`.

The monitoring timeout is a total session deadline, not an inactivity timeout. Quiet monitoring cannot be stopped by an event callback until an event arrives; there is no cross-thread cancellation interface initially. An irreversible LP ALERT terminates monitoring into the sleep state. Idle unsolicited alerts are observed during operations/synchronization, not by a hidden listener.

## Timeouts, synchronization, and special operations

`None` uses configured defaults: connect 5 s, ordinary/monitor operation 10 s, recovery 1 s, synchronization probe 10 ms. Line limit is 4096 bytes; transaction limit is 1 MiB. A zero timeout expires before starting a command write. Reads and partial writes use the remaining deadline, not a new timeout per byte. Blocking DNS resolution precedes the bounded TCP connection attempts; portable serial open/configuration and OS baud-setting calls cannot themselves be reliably interrupted.

Plain connect does not presume a ready chip or known runtime defaults. The first operation drains startup output, tries a single ignored/interrupting space, and, if no prompt arrives, uses a read-only ATRV probe. It consumes delayed startup/interrupt prompts until the voltage reply arrives, so a startup banner cannot be mistaken for a command reply. If no firmware identity was observed, it then sends ATI. `resynchronize` explicitly requests the same bounded synchronization. This never sends a bare CR that could repeat a destructive previous request.

Timeout errors retain partial wire data and a recovery outcome. Recovery consumes a prompt, interrupts when necessary, or marks the session unusable until disconnect/connect. No request is automatically retried. A write timeout can mean the command partially or fully reached the chip. Complete bridge rejections/errors at a prompt leave the session available; reset events invalidate setting assumptions.

BRD is serial-only. The client handles the old-baud ACK, switches host baud, verifies the identity at the new baud, sends handshake CR, and verifies the final ACK. Failure restores the old host baud and attempts bounded synchronization. Successful negotiated baud is retained for reconnect; AT WS retains it, while AT Z uses the original startup configuration or a known enabled PP 0C. If the adapter was already negotiated before construction, supply accurate startup assumptions or avoid full reset. A physical external reset may require creating a client with the adapter's restored baud.

LP accepts the deliberately unterminated OK. `wake` waits for standby entry when necessary, sends a wake character, and synchronizes without reinitialization. Standby entry delay defaults to 1 s; only an emulator should shorten `standby_delay`. Alert-induced sleep uses the documented 2 s transition. A sleep state is not ready for ordinary requests.

Use `set_framing` for a known custom PP CR/LF configuration. Custom framing persists across reconnect. Tracked PPS/PP values determine framing activation on warm/full reset; resetting with an enabled but unknown framing PP is rejected until PPS is read. NULL or prompt-character terminators, equal CR/LF values, and command strings containing the chosen terminator are not supported.

## Firmware and platforms

The following tables describe implemented behavior and validation performed so far.
They do not establish compatibility with every adapter, ECU, or platform.

| Behavior | ELM327 v2.0 | ELM327 v2.1 |
|---|---|---|
| Typed command families inherited through v2.0 | Supported | Supported |
| CTM1/CTM5 | Locally rejected | Supported |
| JTM1/JTM5 | Separate J1939 timer multiplier | Aliases affecting all CAN protocols |
| PP 1E and 28 | Locally rejected | Supported |
| Chip automatic response-pending timeout extension | Not assumed | Documented feature |

Unknown firmware remains unknown; identify before version-specific operations, or explicitly use `assume_firmware` for an emulator. This override persists across reconnect and can be cleared with Unknown. Firmware identification does not prove clone compatibility. No older firmware support is claimed.

| Platform | Transports | Validation |
|---|---|---|
| macOS | Serial, TCP | Host tests, serial mapping, local TCP integration |
| Linux | Serial, TCP | Cross-compilation locally |
| Windows | Serial, TCP | Not run on this host |
| Android | TCP only | aarch64 compile checks with/without default features; runtime device test not performed |

Disable the default serial feature with `--no-default-features` for a dependency-free TCP library. Android gates serial2 out even with defaults enabled and returns UnsupportedTransport for Serial. Linux pseudo-terminal tests do not require hardware. serial2's pseudo-terminal helper is not used on macOS, where it failed with an ioctl error during validation. Physical adapter/ECU validation remains separate from deterministic emulator tests.

## Validation and references

Validation currently relies primarily on automated tests and emulator-based checks,
with limited physical-adapter testing. Successful vehicle requests and recording
with a real ECU remain unverified. Passing tests or compiling for a platform does
not establish readiness for production use.

Run `cargo test`, `cargo test --no-default-features`, `cargo fmt --check`, and `cargo clippy --all-targets -- -D warnings` (also without default features). Tests needing local sockets require permission to bind loopback addresses in a restricted environment. Minimum Rust version is 1.85 (edition 2024).

Bridge syntax is based on the supplied datasheet. Serial APIs were verified against [serial2's documentation](https://docs.rs/serial2/0.2.38/serial2/). Initial OBD formulas/layouts beyond datasheet examples were corroborated against the upstream [python-OBD command definitions](https://github.com/brendan-w/python-OBD/blob/master/obd/commands.py), [decoders](https://github.com/brendan-w/python-OBD/blob/master/obd/decoders.py), and [CAN protocol parser](https://github.com/brendan-w/python-OBD/blob/master/obd/protocols/protocol_can.py). These are implementation references, not replacements for SAE/ISO application standards; no external implementation code is copied into this library. Future PID additions should verify their application definitions explicitly.


## Recording viewer

A native GPUI Kit viewer is available under [`src/viewerui`](src/viewerui/README.md).
Use [`./build_viewerui.sh`](build_viewerui.sh) to build in release mode and run
on the host. Optional `windows`, `linux`, or `macos` selects an OS while retaining
the host CPU; foreign targets are built without launching. `--build-only` skips
launching native builds as well. Cross-builds require the target's linker and SDK.
Run `cargo run --locked --manifest-path src/viewerui/Cargo.toml -- trip.kittx`.
It opens recordings read-only and provides signal selection, linked unit plots,
zoom/pan, sample inspection, and timed DTC observations. GUI dependencies are
isolated from ordinary library builds.

## KittxLauncher

[KittxLauncher](src/kittxlauncher/README.md) opens View, Record, Analyze, or
Real-Time Probe inside its own process and returns when that tool window closes.
Build and run it with `./build_launcher.sh` (`--build-only` skips startup). The
four subprojects retain their standalone binaries and expose reusable UI modules.
The launcher and standalone UIs share a per-user process lock, and the launcher
allows one tool window at a time, preventing competing serial connections.

## KittxRecorder trip recorder

[KittxRecorder](src/kittxrecorder/README.md) is a separate native desktop binary for
fixed-duration or start/stop recording. Select measurements and DTC categories,
choose a new `.kittx` save file, and record over TCP or serial. Files open in ViewerUI.
Dark is the default theme; light mode is also available.

```sh
cargo run --locked --manifest-path src/kittxrecorder/Cargo.toml
```

Build and run a release executable with `./build_recorder.sh`. An optional `windows`,
`linux`, or `macos` argument selects the OS while retaining the host CPU architecture.
Foreign targets are built without launching. Use `--build-only` to skip launching
native builds as well.
See the [recorder build guide](src/kittxrecorder/README.md#run-and-build) for output
paths and platform requirements.

## KittxAnalyzer

[KittxAnalyzer](src/kittxanalyzer/README.md) is a native GPUI Kit UI for selecting
recorded measurements and DTCs from a `.kittx` file. It provides a scrolling
checklist, a report panel, and dark/light themes with dark as the default.
The report panel's **Save report…** button exports the AI answer as Markdown.
The Analyze button sends a prompt containing only selected measurements and DTC
observations to OpenAI, Anthropic, or OpenRouter and displays the returned AI
report. AI-generated reports are experimental and require independent review.
Set the selected service's `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, or
`OPENROUTER_API_KEY` environment variable before launching. The prompt stays out
of the UI and console. Recordings are opened read-only; see the
[AI setup guide](src/kittxanalyzer/README.md#ai-service-setup) for model overrides.
OpenRouter defaults to Nemotron 3 Ultra's free endpoint and enforces free models
with zero provider price caps.

```sh
cargo run --locked --manifest-path src/kittxanalyzer/Cargo.toml
```

Use `./build_analyzer.sh` to build a release executable for the host and run it.
An optional `windows`, `linux`, or `macos` argument selects the target OS while
retaining the host CPU architecture; foreign targets are built without launching.
Add `--build-only` to skip launching a native build. See the
[analyzer build guide](src/kittxanalyzer/README.md#run-and-build) for output paths
and cross-compilation requirements.

## Statement of AI-Assisted Code Generation

This experimental software was conceived, architected, designed, directed, and verified by Erdet Nasufi, with AI-assisted code generation performed using OpenAI's GPT-6.1-Sol.

The human creator maintained intellectual and technical direction throughout the development process, including defining the original concept, system architecture, software design, functional requirements, implementation strategy, and technical specifications.

The AI model served as a code-generation and implementation tool, operating under the explicit direction and technical supervision of the human creator.

All architectural decisions, design principles, technical validation, code review, integration, and final acceptance remained under human responsibility.
