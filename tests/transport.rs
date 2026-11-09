use kittx::{ErrorKind, SerialConfig, TransportConfig, transport::*};
use std::{io, time::Duration};

#[test]
fn serial_and_tcp_configuration_validation() {
    let c = SerialConfig::new("COM12", 38400);
    assert_eq!(c.baud_rate, 38400);
    assert_eq!(c.data_bits, DataBits::Eight);
    assert_eq!(c.parity, Parity::None);
    assert_eq!(c.stop_bits, StopBits::One);
    assert_eq!(c.flow_control, FlowControl::None);
    c.validate().unwrap();
    assert!(SerialConfig::new("", 0).validate().is_err());
    assert!(TransportConfig::tcp("localhost", 35000).validate().is_ok());
    assert!(TransportConfig::tcp("::1", 35000).validate().is_ok());
    assert!(TransportConfig::tcp("", 35000).validate().is_err());
    assert!(TransportConfig::tcp("localhost", 0).validate().is_err());
    TransportConfig::Serial(c).validate().unwrap();
}
struct ByteStream;
impl Transport for ByteStream {
    fn read(&mut self, b: &mut [u8], _: Duration) -> io::Result<usize> {
        b[0] = 42;
        Ok(1)
    }
    fn write(&mut self, b: &[u8], _: Duration) -> io::Result<usize> {
        Ok(b.len())
    }
}
#[test]
fn transport_defaults_and_system_connector_zero_timeout() {
    let mut t = ByteStream;
    let mut b = [0];
    assert_eq!(t.read(&mut b, Duration::from_secs(1)).unwrap(), 1);
    assert_eq!(b, [42]);
    assert_eq!(t.write(&b, Duration::from_secs(1)).unwrap(), 1);
    assert_eq!(t.baud_rate(), None);
    assert_eq!(
        t.set_baud_rate(38400).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    let mut c = SystemConnector;
    let e = match c.open(&TransportConfig::tcp("localhost", 35000), Duration::ZERO) {
        Ok(_) => panic!("must not connect"),
        Err(e) => e,
    };
    assert_eq!(e.kind, ErrorKind::Timeout);
}
#[cfg(not(all(feature = "serial", not(target_os = "android"))))]
#[test]
fn serial_disabled_is_explicitly_unsupported() {
    let mut c = SystemConnector;
    let result = c.open(
        &TransportConfig::Serial(SerialConfig::new("COM1", 38400)),
        Duration::from_secs(1),
    );
    assert_eq!(result.err().unwrap().kind, ErrorKind::UnsupportedTransport);
}
