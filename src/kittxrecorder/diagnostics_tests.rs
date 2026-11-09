use super::*;
use kittx::{ErrorKind, Recovery, obd::Sender};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
#[cfg(target_os = "macos")]
fn macos_dialin_device_resolves_to_existing_callout_without_changing_settings() {
    use kittx::SerialConfig;
    let mut serial = SerialConfig::new("/dev/tty.usbserial-1130", 38400);
    serial.flow_control = kittx::transport::FlowControl::Hardware;
    let requested = TransportConfig::Serial(serial.clone());
    let actual = macos_callout_config(&requested, |path| {
        assert_eq!(path, Path::new("/dev/cu.usbserial-1130"));
        true
    });
    serial.path = "/dev/cu.usbserial-1130".into();
    assert_eq!(actual, TransportConfig::Serial(serial));
    assert_eq!(macos_callout_config(&requested, |_| false), requested);
    for unchanged in [
        TransportConfig::tcp("localhost", 36000),
        TransportConfig::Serial(SerialConfig::new("/dev/cu.usbserial-1130", 38400)),
        TransportConfig::Serial(SerialConfig::new("/tmp/tty.usbserial-1130", 38400)),
        TransportConfig::Serial(SerialConfig::new("/dev/tty.usbserial-1130/extra", 38400)),
    ] {
        assert_eq!(
            macos_callout_config(&unchanged, |_| panic!("no alias lookup needed")),
            unchanged
        );
    }
}

#[test]
#[ignore = "Requires an attached ELM327; set KITTX_SERIAL_DEVICE to the tty device"]
#[cfg(all(target_os = "macos", feature = "serial"))]
fn physical_recorder_connector_initializes_using_dialin_input() {
    use kittx::{Client, ClientOptions, Initialization, Protocol, SerialConfig};
    let path = std::env::var("KITTX_SERIAL_DEVICE").expect("KITTX_SERIAL_DEVICE required");
    let directory = Directory::new();
    let log = directory.log(true);
    let mut client = Client::with_connector(
        TransportConfig::Serial(SerialConfig::new(path, 38400)),
        ClientOptions::default(),
        Initialization::obd(Protocol::Automatic),
        Box::new(DiagnosticConnector { log: log.clone() }),
    )
    .unwrap();
    client.connect(Some(Duration::from_secs(10)), |result| result.unwrap());
    assert!(client.identity().is_some());
    client.read_voltage(Some(Duration::from_secs(2)), |result| {
        assert!(matches!(
            result.unwrap().value,
            kittx::ResponseValue::Voltage(_)
        ));
    });
    client.disconnect(None, |result| result.unwrap());
    let text = std::fs::read_to_string(&log.path).unwrap();
    assert!(text.contains("Using macOS callout device"));
    assert!(text.contains("resolved Serial(SerialConfig { path: \"/dev/cu."));
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kittx-diagnostics-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn log(&self, debug: bool) -> Arc<SessionLog> {
        SessionLog::open(&self.0.join("trip.kittx"), debug).unwrap()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ByteStream {
    baud: u32,
}
impl Transport for ByteStream {
    fn read(&mut self, bytes: &mut [u8], _: Duration) -> io::Result<usize> {
        let response = b"12.4V\r\n>\x00";
        bytes[..response.len()].copy_from_slice(response);
        Ok(response.len())
    }
    fn write(&mut self, bytes: &[u8], _: Duration) -> io::Result<usize> {
        Ok(bytes.len().min(2))
    }
    fn baud_rate(&self) -> Option<u32> {
        Some(self.baud)
    }
    fn set_baud_rate(&mut self, baud: u32) -> io::Result<()> {
        self.baud = baud;
        Ok(())
    }
}

#[test]
fn wire_diagnostics_preserve_bytes_partial_writes_and_baud() {
    let directory = Directory::new();
    let log = directory.log(true);
    let mut transport = DiagnosticTransport {
        inner: Box::new(ByteStream { baud: 38400 }),
        log: log.clone(),
    };
    assert_eq!(
        transport.write(b"ATRV\r", Duration::from_secs(1)).unwrap(),
        2
    );
    let mut response = [0; 32];
    let count = transport
        .read(&mut response, Duration::from_secs(1))
        .unwrap();
    assert_eq!(&response[..count], b"12.4V\r\n>\x00");
    assert_eq!(transport.baud_rate(), Some(38400));
    transport.set_baud_rate(9600).unwrap();
    assert_eq!(transport.baud_rate(), Some(9600));
    let text = std::fs::read_to_string(&log.path).unwrap();
    assert!(text.contains("TX accepted=2/5 len=2 ascii=\"AT\" hex=[41 54]"));
    assert!(text.contains("ascii=\"12.4V\\r\\n>\\x00\""));
    assert!(text.contains("utc_ms=") && text.contains("timeout=1s"));
}

#[test]
fn info_logs_dump_bounded_recent_context_on_failure_and_append() {
    let directory = Directory::new();
    let log = directory.log(false);
    log.info("First session");
    for index in 0..40 {
        log.wire(format!("chunk-{index:02}"));
    }
    assert!(
        !std::fs::read_to_string(&log.path)
            .unwrap()
            .contains("chunk-")
    );
    assert_eq!(log.output.lock().unwrap().recent.len(), RECENT_LIMIT);
    log.failure("No reply to 0902");
    let text = std::fs::read_to_string(&log.path).unwrap();
    assert!(!text.contains("chunk-07"));
    assert!(text.contains("chunk-08") && text.contains("chunk-39"));
    assert!(text.contains("ERROR No reply to 0902"));
    drop(log);
    let log = directory.log(false);
    log.info("Second session");
    let text = std::fs::read_to_string(&log.path).unwrap();
    assert!(text.contains("First session") && text.contains("Second session"));
    assert!(bytes(&vec![b'A'; BYTE_LIMIT + 1]).contains("(truncated)"));
}

#[test]
fn nested_errors_include_initialization_cause_recovery_and_partial_wire() {
    let mut cause = kittx::Error::new(ErrorKind::Timeout, "VIN deadline expired");
    cause.partial = b"0902\rSEARCHING...\r".to_vec();
    cause.recovery = Recovery::ReconnectRequired;
    let mut error = kittx::Error::new(ErrorKind::Initialization { step: 1 }, "Echo setup failed");
    error.cause = Some(Box::new(cause));
    let text = describe_error(&DataError::Source(error));
    assert!(text.contains("Initialization { step: 1 }"));
    assert!(text.contains("Timeout: VIN deadline expired"));
    assert!(text.contains("recovery=ReconnectRequired"));
    assert!(text.contains("0902\\rSEARCHING...\\r"));
}

struct PartialSource;
impl DataSource for PartialSource {
    fn connect(&mut self, _: Duration) -> DataResult<SourceInfo> {
        Err(DataError::Source(kittx::Error::new(
            ErrorKind::Io(io::ErrorKind::NotFound),
            "serial device missing",
        )))
    }
    fn read(&mut self, _: &ReadRequest, _: Duration) -> Observation {
        Observation {
            raw_response: Some(b"7E9 03 7F 01 12\r>".to_vec()),
            responses: vec![EcuObservation {
                sender: Sender::Can(0x7E9),
                value: ObservationValue::Failure {
                    kind: "EcuNegative".into(),
                    message: "PID unsupported".into(),
                },
            }],
            ..Default::default()
        }
    }
    fn disconnect(&mut self, _: Duration) -> DataResult<()> {
        Ok(())
    }
}

#[test]
fn connection_and_per_ecu_failures_reach_logs_and_live_status() {
    let directory = Directory::new();
    let log = directory.log(false);
    let control = Arc::new(SessionControl::new(directory.0.join("trip.kittx")));
    let mut source = DiagnosticSource {
        inner: PartialSource,
        log: log.clone(),
        control: control.clone(),
    };
    assert!(source.connect(Duration::from_secs(1)).is_err());
    assert!(
        control
            .snapshot()
            .last_error
            .unwrap()
            .contains("serial device missing")
    );
    let observation = source.read(
        &ReadRequest::Signal(Signal::RawPid(0x62)),
        Duration::from_secs(1),
    );
    assert_eq!(observation.responses.len(), 1);
    assert!(
        control
            .snapshot()
            .last_error
            .unwrap()
            .contains("PID unsupported")
    );
    source.disconnect(Duration::from_secs(1)).unwrap();
    let text = std::fs::read_to_string(&log.path).unwrap();
    assert!(text.contains("RawPid(98)") && text.contains("7E9 03 7F 01 12\\r>"));
    assert!(text.contains("Adapter disconnected"));
}

#[test]
fn failed_log_writes_do_not_change_transport_results() {
    let directory = Directory::new();
    let log = directory.log(true);
    // A read-only handle deterministically simulates a log write failure.
    log.output.lock().unwrap().file = Some(File::open(&log.path).unwrap());
    let mut transport = DiagnosticTransport {
        inner: Box::new(ByteStream { baud: 38400 }),
        log: log.clone(),
    };
    assert_eq!(transport.write(b"AT", Duration::from_secs(1)).unwrap(), 2);
    assert!(log.output.lock().unwrap().failed);
    let mut response = [0; 32];
    assert_eq!(
        transport
            .read(&mut response, Duration::from_secs(1))
            .unwrap(),
        9
    );
}
