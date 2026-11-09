//! Deadline-bounded byte streams. Blocking DNS and serial opening are OS calls
//! which cannot be reliably interrupted by a portable connect timeout.
use crate::{Error, ErrorKind, Result};
use std::{
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataBits {
    Five,
    Six,
    Seven,
    Eight,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Odd,
    Even,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopBits {
    One,
    Two,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControl {
    None,
    Software,
    Hardware,
}

/// Serial configuration, available as data even when serial support is disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialConfig {
    pub path: PathBuf,
    pub baud_rate: u32,
    pub data_bits: DataBits,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub flow_control: FlowControl,
}
impl SerialConfig {
    /// ELM 8N1 configuration with no flow control; no port is opened.
    pub fn new(path: impl Into<PathBuf>, baud_rate: u32) -> Self {
        Self {
            path: path.into(),
            baud_rate,
            data_bits: DataBits::Eight,
            parity: Parity::None,
            stop_bits: StopBits::One,
            flow_control: FlowControl::None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.path.as_os_str().is_empty() || self.baud_rate == 0 {
            return Err(Error::invalid("serial path and nonzero baud required"));
        }
        Ok(())
    }
}

/// Choose serial or a raw TCP stream. Android rejects Serial explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportConfig {
    Serial(SerialConfig),
    Tcp { host: String, port: u16 },
}
impl TransportConfig {
    pub fn tcp(host: impl Into<String>, port: u16) -> Self {
        Self::Tcp {
            host: host.into(),
            port,
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Serial(c) => c.validate(),
            Self::Tcp { host, port } if host.trim().is_empty() || *port == 0 => {
                Err(Error::invalid("TCP host and nonzero port required"))
            }
            _ => Ok(()),
        }
    }
}

/// Custom transports must honor each timeout. A zero-byte read means EOF,
/// not a timeout; use TimedOut/WouldBlock instead. No library worker is created.
pub trait Transport: Send {
    fn read(&mut self, bytes: &mut [u8], timeout: Duration) -> io::Result<usize>;
    fn write(&mut self, bytes: &[u8], timeout: Duration) -> io::Result<usize>;
    fn baud_rate(&self) -> Option<u32> {
        None
    }
    fn set_baud_rate(&mut self, _baud: u32) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not serial"))
    }
}

/// Opens a fresh connection; injectable for deterministic emulator tests.
pub trait Connector: Send {
    fn open(&mut self, config: &TransportConfig, timeout: Duration) -> Result<Box<dyn Transport>>;
}

#[derive(Debug, Default)]
pub struct SystemConnector;
impl Connector for SystemConnector {
    fn open(&mut self, config: &TransportConfig, timeout: Duration) -> Result<Box<dyn Transport>> {
        config.validate()?;
        if timeout.is_zero() {
            return Err(Error::new(ErrorKind::Timeout, "connect deadline expired"));
        }
        match config {
            TransportConfig::Tcp { host, port } => {
                // DNS resolution precedes the bounded connection attempts.
                let addresses: Vec<_> = (host.as_str(), *port)
                    .to_socket_addrs()
                    .map_err(Error::io)?
                    .collect();
                let deadline = Instant::now()
                    .checked_add(timeout)
                    .ok_or_else(|| Error::invalid("timeout too large"))?;
                let mut last = Error::new(
                    ErrorKind::Io(io::ErrorKind::AddrNotAvailable),
                    "no resolved TCP addresses",
                );
                for address in addresses {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(Error::new(ErrorKind::Timeout, "connect deadline expired"));
                    }
                    match TcpStream::connect_timeout(&address, remaining) {
                        Ok(stream) => {
                            stream.set_nodelay(true).map_err(Error::io)?;
                            return Ok(Box::new(TcpTransport(stream)));
                        }
                        Err(e) => last = Error::io(e),
                    }
                }
                Err(last)
            }
            TransportConfig::Serial(c) => open_serial(c),
        }
    }
}
struct TcpTransport(TcpStream);
impl Transport for TcpTransport {
    fn read(&mut self, bytes: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.0.set_read_timeout(Some(timeout))?;
        self.0.read(bytes)
    }
    fn write(&mut self, bytes: &[u8], timeout: Duration) -> io::Result<usize> {
        self.0.set_write_timeout(Some(timeout))?;
        self.0.write(bytes)
    }
}

#[cfg(all(feature = "serial", not(target_os = "android")))]
fn open_serial(c: &SerialConfig) -> Result<Box<dyn Transport>> {
    let port = serial2::SerialPort::open(&c.path, |settings| configure_serial(c, settings))
        .map_err(Error::io)?;
    #[cfg(target_os = "macos")]
    apply_standard_serial_baud(&port, c.baud_rate).map_err(Error::io)?;
    Ok(Box::new(SerialTransport {
        port,
        baud: c.baud_rate,
    }))
}

// serial2 applies a temporary 9600 baud before IOSSIOSPEED on macOS. Some USB
// drivers accept that ioctl without applying the requested rate. For rates
// supported by termios, reapply the speed through the standard driver path.
// Keep serial2's IOSSIOSPEED behavior for nonstandard BRD divisor rates.
#[cfg(all(feature = "serial", target_os = "macos"))]
fn apply_standard_serial_baud(port: &serial2::SerialPort, baud: u32) -> io::Result<()> {
    use nix::sys::termios::{self, BaudRate, SetArg};
    if BaudRate::try_from(nix::libc::speed_t::from(baud)).is_err() {
        return Ok(());
    }
    let mut settings = termios::tcgetattr(port)?;
    termios::cfsetspeed(&mut settings, baud)?;
    termios::tcsetattr(port, SetArg::TCSANOW, &settings).map_err(Into::into)
}

#[cfg(all(feature = "serial", not(target_os = "android")))]
fn configure_serial(
    c: &SerialConfig,
    mut settings: serial2::Settings,
) -> io::Result<serial2::Settings> {
    let (bits, parity, stop, flow) = serial_values(c);
    settings.set_raw();
    settings.set_baud_rate(c.baud_rate)?;
    settings.set_char_size(bits);
    settings.set_parity(parity);
    settings.set_stop_bits(stop);
    settings.set_flow_control(flow);
    Ok(settings)
}
#[cfg(all(feature = "serial", not(target_os = "android")))]
fn serial_values(
    c: &SerialConfig,
) -> (
    serial2::CharSize,
    serial2::Parity,
    serial2::StopBits,
    serial2::FlowControl,
) {
    (
        match c.data_bits {
            DataBits::Five => serial2::CharSize::Bits5,
            DataBits::Six => serial2::CharSize::Bits6,
            DataBits::Seven => serial2::CharSize::Bits7,
            DataBits::Eight => serial2::CharSize::Bits8,
        },
        match c.parity {
            Parity::None => serial2::Parity::None,
            Parity::Odd => serial2::Parity::Odd,
            Parity::Even => serial2::Parity::Even,
        },
        match c.stop_bits {
            StopBits::One => serial2::StopBits::One,
            StopBits::Two => serial2::StopBits::Two,
        },
        match c.flow_control {
            FlowControl::None => serial2::FlowControl::None,
            FlowControl::Software => serial2::FlowControl::XonXoff,
            FlowControl::Hardware => serial2::FlowControl::RtsCts,
        },
    )
}
#[cfg(not(all(feature = "serial", not(target_os = "android"))))]
fn open_serial(_: &SerialConfig) -> Result<Box<dyn Transport>> {
    Err(Error::new(
        ErrorKind::UnsupportedTransport,
        "serial requires the serial feature on a desktop target",
    ))
}

#[cfg(all(feature = "serial", not(target_os = "android")))]
struct SerialTransport {
    port: serial2::SerialPort,
    baud: u32,
}
#[cfg(all(feature = "serial", not(target_os = "android")))]
impl Transport for SerialTransport {
    fn read(&mut self, bytes: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.port.set_read_timeout(timeout)?;
        self.port.read(bytes)
    }
    fn write(&mut self, bytes: &[u8], timeout: Duration) -> io::Result<usize> {
        self.port.set_write_timeout(timeout)?;
        self.port.write(bytes)
    }
    fn baud_rate(&self) -> Option<u32> {
        Some(self.baud)
    }
    fn set_baud_rate(&mut self, baud: u32) -> io::Result<()> {
        let mut c = self.port.get_configuration()?;
        c.set_baud_rate(baud)?;
        self.port.set_configuration(&c)?;
        #[cfg(target_os = "macos")]
        apply_standard_serial_baud(&self.port, baud)?;
        self.baud = baud;
        Ok(())
    }
}

#[cfg(all(test, feature = "serial", not(target_os = "android")))]
mod tests {
    use super::*;
    #[test]
    #[cfg(target_os = "macos")]
    fn macos_standard_baud_preserves_raw_settings_and_custom_rates() {
        use nix::sys::termios::{self, SetArg};
        // serial2's pair helper uses an ioctl unsupported by macOS PTYs.
        let pair = nix::pty::openpty(None, None).unwrap();
        let port = serial2::SerialPort::from(pair.slave);
        let mut settings = termios::tcgetattr(&port).unwrap();
        termios::cfmakeraw(&mut settings);
        termios::cfsetspeed(&mut settings, 9600_u32).unwrap();
        termios::tcsetattr(&port, SetArg::TCSANOW, &settings).unwrap();
        let before = termios::tcgetattr(&port).unwrap();
        for baud in [38400, 115200, 9600] {
            apply_standard_serial_baud(&port, baud).unwrap();
            let after = termios::tcgetattr(&port).unwrap();
            assert_eq!(termios::cfgetispeed(&after), baud);
            assert_eq!(termios::cfgetospeed(&after), baud);
            assert_eq!(after.input_flags, before.input_flags);
            assert_eq!(after.output_flags, before.output_flags);
            assert_eq!(after.control_flags, before.control_flags);
            assert_eq!(after.local_flags, before.local_flags);
            assert_eq!(after.control_chars, before.control_chars);
        }
        // BRD's approximate 115200 divisor rate must keep the IOSSIOSPEED path.
        apply_standard_serial_baud(&port, 4_000_000 / 35).unwrap();
        assert_eq!(
            termios::cfgetospeed(&termios::tcgetattr(&port).unwrap()),
            9600
        );
    }

    #[test]
    fn every_serial_setting_maps_to_serial2() {
        for (bits, expected) in [
            (DataBits::Five, serial2::CharSize::Bits5),
            (DataBits::Six, serial2::CharSize::Bits6),
            (DataBits::Seven, serial2::CharSize::Bits7),
            (DataBits::Eight, serial2::CharSize::Bits8),
        ] {
            let mut config = SerialConfig::new("pty", 38400);
            config.data_bits = bits;
            assert_eq!(serial_values(&config).0, expected);
        }
        for (parity, expected) in [
            (Parity::None, serial2::Parity::None),
            (Parity::Odd, serial2::Parity::Odd),
            (Parity::Even, serial2::Parity::Even),
        ] {
            let mut config = SerialConfig::new("pty", 38400);
            config.parity = parity;
            assert_eq!(serial_values(&config).1, expected);
        }
        for (stop_bits, expected) in [
            (StopBits::One, serial2::StopBits::One),
            (StopBits::Two, serial2::StopBits::Two),
        ] {
            let mut config = SerialConfig::new("pty", 38400);
            config.stop_bits = stop_bits;
            assert_eq!(serial_values(&config).2, expected);
        }
        for (flow_control, expected) in [
            (FlowControl::None, serial2::FlowControl::None),
            (FlowControl::Software, serial2::FlowControl::XonXoff),
            (FlowControl::Hardware, serial2::FlowControl::RtsCts),
        ] {
            let mut config = SerialConfig::new("pty", 38400);
            config.flow_control = flow_control;
            assert_eq!(serial_values(&config).3, expected);
        }
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn serial_transport_uses_real_pty_reads_writes_and_baud_changes() {
        let (mut peer, port) = serial2::SerialPort::pair().unwrap();
        let mut transport = SerialTransport { port, baud: 38400 };
        let config = SerialConfig::new("pty", 38400);
        let settings =
            configure_serial(&config, transport.port.get_configuration().unwrap()).unwrap();
        transport.port.set_configuration(&settings).unwrap();
        peer.set_read_timeout(Duration::from_secs(1)).unwrap();
        peer.write_all(b"X").unwrap();
        let mut byte = [0];
        assert_eq!(
            transport.read(&mut byte, Duration::from_secs(1)).unwrap(),
            1
        );
        assert_eq!(byte, [b'X']);
        assert_eq!(transport.write(b"Y", Duration::from_secs(1)).unwrap(), 1);
        peer.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'Y']);
        transport.set_baud_rate(115200).unwrap();
        assert_eq!(transport.baud_rate(), Some(115200));
    }
}
