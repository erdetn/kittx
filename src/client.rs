use crate::{
    command::*,
    error::*,
    framing::{Decoder, FrameEvent},
    obd::{self, EcuMessage, Layout},
    response::{self, *},
    transport::{Connector, SystemConnector, Transport, TransportConfig},
};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    time::{Duration, Instant},
};

/// Limits and deadlines. Ordinary requests default to 10 s to allow searching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientOptions {
    pub connect_timeout: Duration,
    pub operation_timeout: Duration,
    pub recovery_timeout: Duration,
    pub synchronization_probe: Duration,
    pub max_line_bytes: usize,
    pub max_transaction_bytes: usize,
    pub capture_raw: bool,
    /// LP's delayed entry into standby; set shorter only for an emulator.
    pub standby_delay: Duration,
}
impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            operation_timeout: Duration::from_secs(10),
            recovery_timeout: Duration::from_secs(1),
            synchronization_probe: Duration::from_millis(10),
            max_line_bytes: 4096,
            max_transaction_bytes: 1024 * 1024,
            capture_raw: true,
            standby_delay: Duration::from_secs(1),
        }
    }
}
impl ClientOptions {
    pub fn validate(&self) -> Result<()> {
        if self.max_line_bytes == 0
            || self.max_transaction_bytes < self.max_line_bytes
            || self.synchronization_probe.is_zero()
        {
            return Err(Error::invalid("invalid client limits/probe"));
        }
        for duration in [
            self.connect_timeout,
            self.operation_timeout,
            self.recovery_timeout,
            self.synchronization_probe,
            self.standby_delay,
        ] {
            Deadline::new(duration)?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Disconnected,
    Unsynchronized,
    Ready,
    Asleep,
    Unusable,
}

/// Runtime configuration: None means unobserved or invalidated, not factory default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub echo: Option<bool>,
    pub linefeeds: Option<bool>,
    pub spaces: Option<bool>,
    pub headers: Option<bool>,
    pub dlc: Option<bool>,
    pub can_formatting: Option<bool>,
    pub responses: Option<bool>,
    pub long_messages: Option<bool>,
    pub variable_dlc: Option<bool>,
    pub extended_address: Option<Option<u8>>,
    pub tester_address: Option<u8>,
    pub protocol: Option<Protocol>,
    pub active_protocol: Option<bool>,
    pub automatic: Option<bool>,
    pub timer_multiplier: Option<u8>,
    pub j1939_timer_multiplier: Option<u8>,
    pub serial_baud: Option<u32>,
    pub cr: u8,
    pub lf: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            echo: None,
            linefeeds: None,
            spaces: None,
            headers: None,
            dlc: None,
            can_formatting: None,
            responses: None,
            long_messages: None,
            variable_dlc: None,
            extended_address: None,
            tester_address: None,
            protocol: None,
            active_protocol: None,
            automatic: None,
            timer_multiplier: None,
            j1939_timer_multiplier: None,
            serial_baud: None,
            cr: b'\r',
            lf: b'\n',
        }
    }
}

/// Ordered opt-in steps executed by connect. No implicit reset or EEPROM writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Initialization {
    pub steps: Vec<Command>,
}
impl Initialization {
    pub fn new(steps: Vec<Command>) -> Self {
        Self { steps }
    }
    /// Identifies firmware and sets E0,L0,S1,H1,D0,CAF1,R1,CEA(off),TP.
    /// TP does not immediately write EEPROM, but chip M1 may later save a found protocol.
    pub fn obd(protocol: Protocol) -> Self {
        Self::new(vec![
            Command::Fixed(Fixed::Identify),
            Command::Toggle(Toggle::Echo, false),
            Command::Toggle(Toggle::Linefeeds, false),
            Command::Toggle(Toggle::Spaces, true),
            Command::Toggle(Toggle::Headers, true),
            Command::Toggle(Toggle::Dlc, false),
            Command::Toggle(Toggle::CanFormatting, true),
            Command::Toggle(Toggle::Responses, true),
            Command::CanExtendedAddress(None),
            Command::SetProtocol {
                protocol,
                automatic: protocol == Protocol::Automatic,
                persist: false,
            },
        ])
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorControl {
    Continue,
    Stop,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MonitorSummary {
    pub events: usize,
    pub stopped_by_caller: bool,
    pub transaction: Transaction,
}

/// Reassembled ECU replies and their original ECU transaction bytes.
/// Preparatory AT transactions are not included. With raw capture disabled,
/// `raw` is empty. Observation timing brackets the full high-level operation.
#[derive(Debug, Clone)]
pub struct ObdEvidence {
    pub messages: Vec<EcuMessage>,
    pub raw: Vec<u8>,
    pub protocol: Protocol,
}

/// Synchronous session owner. Callbacks execute inline; no scheduling is hidden.
pub struct Client {
    config: TransportConfig,
    options: ClientOptions,
    initialization: Initialization,
    connector: Box<dyn Connector>,
    transport: Option<Box<dyn Transport>>,
    state: SessionState,
    settings: Settings,
    decoder: Decoder,
    firmware: Firmware,
    firmware_override: Option<Firmware>,
    identity: Option<String>,
    last: Option<Command>,
    flow_header: bool,
    flow_data: bool,
    baud_timeout: u8,
    parameters: BTreeMap<u8, (Option<u8>, Option<bool>)>,
    user_formats: [Option<u8>; 2],
    standby_enter_at: Option<Instant>,
    literal_value_line: bool,
    received: VecDeque<u8>,
    framing: (u8, u8),
    startup_baud: Option<u32>,
}
impl Client {
    pub fn new(config: TransportConfig) -> Self {
        Self::build(
            config,
            ClientOptions::default(),
            Initialization::default(),
            Box::new(SystemConnector),
        )
    }
    pub fn with_initialization(config: TransportConfig, initialization: Initialization) -> Self {
        Self::build(
            config,
            ClientOptions::default(),
            initialization,
            Box::new(SystemConnector),
        )
    }
    pub fn with_options(config: TransportConfig, options: ClientOptions) -> Result<Self> {
        config.validate()?;
        options.validate()?;
        Ok(Self::build(
            config,
            options,
            Initialization::default(),
            Box::new(SystemConnector),
        ))
    }
    /// Inject a connector for an emulator or transport implementation. No I/O occurs.
    pub fn with_connector(
        config: TransportConfig,
        options: ClientOptions,
        initialization: Initialization,
        connector: Box<dyn Connector>,
    ) -> Result<Self> {
        config.validate()?;
        options.validate()?;
        Ok(Self::build(config, options, initialization, connector))
    }
    fn build(
        config: TransportConfig,
        options: ClientOptions,
        initialization: Initialization,
        connector: Box<dyn Connector>,
    ) -> Self {
        let decoder =
            Decoder::new(b'\r', b'\n', options.max_line_bytes).expect("validated options");
        let startup_baud = match &config {
            TransportConfig::Serial(c) => Some(c.baud_rate),
            _ => None,
        };
        Self {
            config,
            options,
            initialization,
            connector,
            transport: None,
            state: SessionState::Disconnected,
            settings: Settings::default(),
            decoder,
            firmware: Firmware::Unknown,
            firmware_override: None,
            identity: None,
            last: None,
            flow_header: false,
            flow_data: false,
            baud_timeout: 0x0F,
            parameters: BTreeMap::new(),
            user_formats: [None; 2],
            standby_enter_at: None,
            literal_value_line: false,
            received: VecDeque::new(),
            framing: (b'\r', b'\n'),
            startup_baud,
        }
    }
    pub fn config(&self) -> &TransportConfig {
        &self.config
    }
    pub fn options(&self) -> &ClientOptions {
        &self.options
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    pub fn state(&self) -> SessionState {
        self.state
    }
    pub fn firmware(&self) -> Firmware {
        self.firmware
    }
    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }
    /// Persistent firmware assumption for an emulator with nonstandard identity.
    /// Unknown clears the override and restores the observed identity's version.
    pub fn assume_firmware(&mut self, version: Firmware) {
        self.firmware_override = if version == Firmware::Unknown {
            None
        } else {
            Some(version)
        };
        self.firmware = self.firmware_override.unwrap_or_else(|| {
            self.identity
                .as_deref()
                .map(Firmware::parse)
                .unwrap_or(Firmware::Unknown)
        });
    }
    /// Configure known custom PP terminators without sending commands. Resynchronization
    /// is required afterwards; queued bytes are not decoded using old settings.
    pub fn set_framing(&mut self, cr: u8, lf: u8) -> Result<()> {
        let decoder = Decoder::new(cr, lf, self.options.max_line_bytes)?;
        self.settings.cr = cr;
        self.settings.lf = lf;
        self.framing = (cr, lf);
        self.decoder = decoder;
        self.received.clear();
        if self.transport.is_some() {
            self.state = SessionState::Unsynchronized;
        }
        Ok(())
    }

    pub fn connect(&mut self, timeout: Option<Duration>, callback: impl FnOnce(Result<()>)) {
        let r = self.connect_inner(timeout);
        callback(r);
    }
    fn connect_inner(&mut self, timeout: Option<Duration>) -> Result<()> {
        if self.transport.is_some() {
            return Err(Error::new(
                ErrorKind::AlreadyConnected,
                "disconnect before connecting again",
            ));
        }
        self.config.validate()?;
        self.options.validate()?;
        let deadline = Deadline::new(timeout.unwrap_or(self.options.connect_timeout))?;
        let remaining = deadline.remaining()?;
        let transport = self.connector.open(&self.config, remaining)?;
        let cr = self.settings.cr;
        let lf = self.settings.lf;
        self.settings = Settings {
            cr,
            lf,
            serial_baud: transport.baud_rate(),
            ..Settings::default()
        };
        self.transport = Some(transport);
        self.state = SessionState::Unsynchronized;
        self.firmware = self.firmware_override.unwrap_or(Firmware::Unknown);
        self.identity = None;
        self.last = None;
        self.parameters.clear();
        self.flow_header = false;
        self.flow_data = false;
        self.user_formats = [None; 2];
        self.baud_timeout = 0x0F;
        self.decoder.clear();
        self.standby_enter_at = None;
        self.literal_value_line = false;
        self.received.clear();
        for (index, command) in self.initialization.steps.clone().into_iter().enumerate() {
            let description = format!("{command:?}");
            let r = deadline
                .remaining()
                .and_then(|remaining| self.execute_inner(command, Some(remaining)));
            if let Err(error) = r {
                self.disconnect_inner();
                let mut e = Error::new(
                    ErrorKind::Initialization { step: index },
                    format!("initialization step {index} ({description}) failed: {error}"),
                );
                e.partial = error.partial.clone();
                e.cause = Some(Box::new(error));
                return Err(e);
            }
        }
        Ok(())
    }
    pub fn disconnect(&mut self, _timeout: Option<Duration>, callback: impl FnOnce(Result<()>)) {
        self.disconnect_inner();
        callback(Ok(()));
    }
    fn disconnect_inner(&mut self) {
        self.transport = None;
        self.state = SessionState::Disconnected;
        self.settings = Settings {
            cr: self.framing.0,
            lf: self.framing.1,
            ..Settings::default()
        };
        self.firmware = Firmware::Unknown;
        self.identity = None;
        self.last = None;
        self.parameters.clear();
        self.decoder.clear();
        self.standby_enter_at = None;
        self.literal_value_line = false;
        self.received.clear();
    }

    pub fn execute(
        &mut self,
        command: Command,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Response>),
    ) {
        let r = self.execute_inner(command, timeout).map(|mut response| {
            if !self.options.capture_raw {
                response.transaction.raw.clear();
            }
            response
        });
        callback(r);
    }
    pub fn identify(&mut self, timeout: Option<Duration>, callback: impl FnOnce(Result<Response>)) {
        self.execute(Command::Fixed(Fixed::Identify), timeout, callback);
    }
    pub fn read_voltage(
        &mut self,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Response>),
    ) {
        self.execute(Command::Fixed(Fixed::Voltage), timeout, callback);
    }
    pub fn select_protocol(
        &mut self,
        protocol: Protocol,
        automatic: bool,
        persist: bool,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Response>),
    ) {
        self.execute(
            Command::SetProtocol {
                protocol,
                automatic,
                persist,
            },
            timeout,
            callback,
        );
    }
    pub fn describe_protocol(
        &mut self,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Response>),
    ) {
        self.execute(Command::Fixed(Fixed::DescribeProtocol), timeout, callback);
    }
    pub fn send_vehicle(
        &mut self,
        bytes: Vec<u8>,
        responses: Option<u8>,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Response>),
    ) {
        self.execute(Command::Vehicle { bytes, responses }, timeout, callback);
    }

    fn execute_inner(&mut self, command: Command, timeout: Option<Duration>) -> Result<Response> {
        self.require_connection()?;
        let deadline = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))?;
        deadline.remaining()?;
        command.encode(self.settings.cr)?;
        self.validate_firmware(&command)?;
        if command.expectation() == ResponseExpectation::Stream {
            return Err(Error::invalid("use monitor() for streaming commands"));
        }
        if matches!(&command,Command::RawAt(s) if is_special_raw(s)) {
            return Err(Error::invalid(
                "use typed command for baud, sleep, reset or monitoring",
            ));
        }
        self.validate_session(&command)?;
        if let Err(mut error) = self.synchronize_if_needed(deadline) {
            self.recover_error(&mut error);
            return Err(error);
        }
        let effective = if command == Command::Repeat {
            self.last
                .clone()
                .ok_or_else(|| Error::invalid("no previous command to repeat"))?
        } else {
            command.clone()
        };
        if command == Command::Repeat
            && matches!(
                effective.expectation(),
                ResponseExpectation::Baud
                    | ResponseExpectation::Sleep
                    | ResponseExpectation::Stream
            )
        {
            return Err(Error::invalid(
                "repeat requires an ordinary previous command",
            ));
        }
        if command == Command::Repeat {
            self.validate_session(&effective)?;
            self.validate_firmware(&effective)?;
        }
        let bytes = command.encode(self.settings.cr)?;
        let echo = if self.settings.echo == Some(false) {
            String::new()
        } else {
            String::from_utf8_lossy(&bytes[..bytes.len() - 1]).into_owned()
        };
        let mut transaction = Transaction::default();
        let r = (|| {
            self.write_all(&bytes, deadline)?;
            self.literal_value_line = effective == Command::Fixed(Fixed::Identifier);
            if effective == Command::Fixed(Fixed::Reset) {
                let baud = self.startup_baud;
                self.apply_defaults(true)?;
                if let Some(baud) = baud {
                    let baud = self.pp_baud().unwrap_or(baud);
                    self.transport_mut()?
                        .set_baud_rate(baud)
                        .map_err(Error::io)?;
                    self.settings.serial_baud = Some(baud);
                    self.remember_baud(baud);
                }
            } else if effective == Command::Fixed(Fixed::WarmStart) {
                self.apply_defaults(true)?;
            }
            if effective.expectation() == ResponseExpectation::Baud {
                return self.baud_handshake(&effective, &echo, deadline, &mut transaction);
            }
            if effective.expectation() == ResponseExpectation::Sleep {
                loop {
                    if let Some(event) = self.read_event(&mut transaction, deadline)? {
                        match event {
                            FrameEvent::Line(s) => transaction.add(s, &echo),
                            FrameEvent::Prompt => {
                                return Err(Error::malformed(
                                    "LP returned prompt without standby acknowledgment",
                                ));
                            }
                        };
                        transaction.check()?;
                    }
                    if self.decoder.pending() == b"OK" {
                        self.decoder.clear();
                        self.state = SessionState::Asleep;
                        self.standby_enter_at =
                            Instant::now().checked_add(self.options.standby_delay);
                        return Ok(Response {
                            value: ResponseValue::Asleep,
                            transaction: transaction.clone(),
                        });
                    }
                }
            }
            self.read_prompt(&mut transaction, &echo, deadline)?;
            if effective.expectation() == ResponseExpectation::Bus
                && self.settings.responses != Some(false)
                && !transaction.records.iter().any(|record| {
                    matches!(
                        record,
                        Record::Hex(_)
                            | Record::CanFrame { .. }
                            | Record::Length(_)
                            | Record::Segment { .. }
                            | Record::Unknown(_)
                            | Record::FlowControl(_)
                            | Record::RemoteFrame(_)
                            | Record::Fault(_)
                            | Record::Reset(_)
                            | Record::Identity(_)
                    )
                })
            {
                return Err(Error::malformed(
                    "missing bus response; only known R0 permits empty completion",
                ));
            }
            if !matches!(
                effective,
                Command::Fixed(Fixed::Identify | Fixed::Reset | Fixed::WarmStart)
            ) && transaction
                .records
                .iter()
                .any(|r| matches!(r, Record::Identity(_)))
            {
                self.invalidate();
                return Err(Error::new(
                    ErrorKind::Reset,
                    "unexpected startup identification",
                ));
            }
            if transaction
                .records
                .iter()
                .any(|r| matches!(r, Record::Reset(_)))
            {
                self.invalidate();
            }
            let response = response::decode(&effective, transaction.clone())?;
            self.update(&effective, &response)?;
            self.state = SessionState::Ready;
            if transaction
                .records
                .iter()
                .any(|r| matches!(r,Record::Alert(s) if s.ends_with("LP ALERT")))
            {
                self.state = SessionState::Asleep;
                self.standby_enter_at = Instant::now().checked_add(Duration::from_secs(2));
            }
            Ok(response)
        })();
        match r {
            Ok(response) => {
                self.last = Some(effective);
                Ok(response)
            }
            Err(mut error) => {
                if error.partial.is_empty() {
                    error.partial = transaction.raw;
                }
                if !matches!(self.state, SessionState::Ready | SessionState::Asleep) {
                    self.recover_error(&mut error);
                }
                Err(error)
            }
        }
    }

    fn validate_session(&self, command: &Command) -> Result<()> {
        if matches!(command, Command::Fixed(Fixed::Reset | Fixed::WarmStart)) {
            for id in [0x0D, 0x0A] {
                if let Some((None, Some(true))) = self.parameters.get(&id) {
                    return Err(Error::new(
                        ErrorKind::UnknownCapability,
                        "read PPS before resetting with unknown enabled framing parameters",
                    ));
                }
            }
            let cr = self.reset_character(0x0D, self.settings.cr);
            let lf = self.reset_character(0x0A, self.settings.lf);
            Decoder::new(cr, lf, self.options.max_line_bytes)?;
        }
        if command.expectation() == ResponseExpectation::Baud
            && self
                .transport
                .as_ref()
                .and_then(|t| t.baud_rate())
                .is_none()
        {
            return Err(Error::new(
                ErrorKind::UnsupportedTransport,
                "BRD requires a serial transport",
            ));
        }
        if matches!(command, Command::Fixed(Fixed::FastInit))
            && self
                .settings
                .protocol
                .is_some_and(|p| p != Protocol::KwpFast)
        {
            return Err(Error::invalid("FI requires protocol 5"));
        }
        if matches!(command, Command::Fixed(Fixed::SlowInit))
            && self
                .settings
                .protocol
                .is_some_and(|p| !matches!(p, Protocol::Iso9141 | Protocol::KwpSlow))
        {
            return Err(Error::invalid("SI requires protocol 3/4"));
        }
        if let Command::FlowMode(mode) = command {
            if *mode != FlowMode::Automatic && !self.flow_data {
                return Err(Error::invalid("define flow-control data first"));
            }
            if *mode == FlowMode::UserHeaderAndData && !self.flow_header {
                return Err(Error::invalid("define flow-control header first"));
            }
        }
        if matches!(command, Command::Fixed(Fixed::RemoteRequest))
            && !(self.settings.protocol.is_some_and(Protocol::is_can)
                && self.settings.active_protocol == Some(true))
        {
            return Err(Error::new(
                ErrorKind::UnknownCapability,
                "RTR requires a known active CAN protocol; send a request or explicitly bypass initialization first",
            ));
        }
        if let Command::Vehicle { bytes, .. } = command {
            let p = self.settings.protocol;
            let no_pci = p == Some(Protocol::J1939)
                || matches!(p, Some(Protocol::User1 | Protocol::User2))
                    && self
                        .user_format(p.expect("user protocol"))
                        .is_some_and(|format| format == 0 || format == 2);
            let max = if no_pci {
                8
            } else if p.is_some_and(Protocol::is_can) {
                let address = usize::from(self.settings.extended_address.flatten().is_some());
                if self.settings.can_formatting == Some(false) {
                    8 - address
                } else {
                    7 - address
                }
            } else if p.is_some_and(|p| p != Protocol::Automatic)
                && self.settings.long_messages == Some(true)
            {
                8
            } else {
                7
            };
            if bytes.len() > max {
                return Err(Error::invalid(format!(
                    "current frame capacity is {max} payload bytes"
                )));
            }
        }
        Ok(())
    }
    fn validate_firmware(&self, command: &Command) -> Result<()> {
        if command.requires_v21() {
            match self.firmware {
                Firmware::V20 => {
                    return Err(Error::new(
                        ErrorKind::UnsupportedFirmware,
                        "command requires ELM327 v2.1",
                    ));
                }
                Firmware::Unknown => {
                    return Err(Error::new(
                        ErrorKind::UnknownCapability,
                        "identify or explicitly assume v2.1 first",
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn reset_character(&self, id: u8, current: u8) -> u8 {
        match self.parameters.get(&id) {
            Some((Some(value), Some(true))) => *value,
            Some((_, Some(false))) => {
                if id == 0x0D {
                    b'\r'
                } else {
                    b'\n'
                }
            }
            _ => current,
        }
    }
    fn require_connection(&self) -> Result<()> {
        match self.state {
            SessionState::Disconnected => Err(Error::new(ErrorKind::Disconnected, "connect first")),
            SessionState::Unusable => Err(Error::new(
                ErrorKind::UnusableSession,
                "disconnect/connect required",
            )),
            SessionState::Asleep => Err(Error::new(
                ErrorKind::UnusableSession,
                "wake before invoking another command",
            )),
            _ => Ok(()),
        }
    }
    fn transport_mut(&mut self) -> Result<&mut (dyn Transport + 'static)> {
        self.transport
            .as_deref_mut()
            .ok_or_else(|| Error::new(ErrorKind::Disconnected, "connect first"))
    }
    fn write_all(&mut self, mut bytes: &[u8], deadline: Deadline) -> Result<()> {
        self.state = SessionState::Unsynchronized;
        // A partial/rejected command or synchronization probe can change the
        // chip's one-command memory. Only a completed operation restores it.
        self.last = None;
        while !bytes.is_empty() {
            let remaining = deadline.remaining()?;
            match self.transport_mut()?.write(bytes, remaining) {
                Ok(0) => {
                    return Err(Error::new(
                        ErrorKind::Io(io::ErrorKind::WriteZero),
                        "transport wrote zero bytes",
                    ));
                }
                Ok(n) if n <= bytes.len() => bytes = &bytes[n..],
                Ok(_) => return Err(Error::malformed("transport write count exceeds buffer")),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(Error::io(e)),
            }
        }
        Ok(())
    }
    fn read_event(
        &mut self,
        transaction: &mut Transaction,
        deadline: Deadline,
    ) -> Result<Option<FrameEvent>> {
        let remaining = deadline.remaining()?;
        if self.received.is_empty() {
            let mut bytes = [0; 4096];
            match self.transport_mut()?.read(&mut bytes, remaining) {
                Ok(0) => {
                    return Err(Error::new(
                        ErrorKind::Io(io::ErrorKind::UnexpectedEof),
                        "transport closed",
                    ));
                }
                Ok(n) if n <= bytes.len() => self.received.extend(&bytes[..n]),
                Ok(_) => return Err(Error::malformed("transport read count exceeds buffer")),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => return Ok(None),
                Err(e) => return Err(Error::io(e)),
            }
        }
        let byte = self.received.pop_front().expect("nonempty receive buffer");
        if transaction.raw.len() >= self.options.max_transaction_bytes {
            return Err(Error::new(
                ErrorKind::BufferOverflow,
                "transaction limit exceeded",
            ));
        }
        transaction.raw.push(byte);
        if self.literal_value_line {
            self.decoder.push_literal(byte)
        } else {
            self.decoder.push(byte)
        }
    }
    fn read_prompt(
        &mut self,
        transaction: &mut Transaction,
        echo: &str,
        deadline: Deadline,
    ) -> Result<()> {
        loop {
            if let Some(event) = self.read_event(transaction, deadline)? {
                match event {
                    FrameEvent::Line(s) => {
                        let before = transaction.lines.len();
                        transaction.add(s, echo);
                        if transaction.lines.len() != before {
                            self.literal_value_line = false;
                        }
                    }
                    FrameEvent::Prompt => {
                        self.state = SessionState::Ready;
                        return Ok(());
                    }
                }
            }
        }
    }
    fn observe_identity(&mut self, tx: &Transaction) {
        if let Some(s) = tx.lines.iter().find(|s| s.starts_with("ELM327 ")) {
            self.firmware = self.firmware_override.unwrap_or_else(|| Firmware::parse(s));
            self.identity = Some(s.clone());
        }
    }
    fn synchronize_if_needed(&mut self, deadline: Deadline) -> Result<()> {
        if self.state == SessionState::Ready {
            return Ok(());
        }
        // Drain startup output first. Idle emulators need not send a banner.
        let mut tx = Transaction::default();
        let probe = Deadline::new(
            deadline
                .remaining()?
                .min(self.options.synchronization_probe),
        )?;
        match self.read_prompt(&mut tx, "", probe) {
            Ok(()) => {
                self.observe_identity(&tx);
                return tx.check();
            }
            Err(e) if e.kind == ErrorKind::Timeout => {}
            Err(e) => return Err(e),
        }
        // A space interrupts a running transaction, but is ignored in an idle input buffer.
        self.write_all(b" ", deadline)?;
        let probe = Deadline::new(
            deadline
                .remaining()?
                .min(self.options.synchronization_probe),
        )?;
        match self.read_prompt(&mut tx, "", probe) {
            Ok(()) => {
                self.observe_identity(&tx);
                return tx.check();
            }
            Err(e) if e.kind == ErrorKind::Timeout => {}
            Err(e) => return Err(e),
        }
        // A delayed startup banner can arrive after both short probes. ATI has
        // the same reply as that banner: accepting its first prompt would leave
        // the real ATI reply queued and shift every subsequent transaction.
        // ATRV is read-only and its voltage reply distinguishes it from startup.
        self.decoder.clear();
        let command = Command::Fixed(Fixed::Voltage);
        let bytes = command.encode(self.settings.cr)?;
        self.write_all(&bytes, deadline)?;
        loop {
            tx = Transaction::default();
            self.read_prompt(&mut tx, "ATRV", deadline)
                .map_err(|mut error| {
                    error.partial = tx.raw.clone();
                    error
                })?;
            tx.check()?;
            self.observe_identity(&tx);
            // Consume unsolicited startup/interrupt prompts, but keep waiting
            // for the probe's own response within the original total deadline.
            if tx.records.iter().all(|r| {
                matches!(r, Record::Identity(_)) || matches!(r, Record::Status(s) if s == "STOPPED")
            }) {
                continue;
            }
            response::decode(&command, tx)?;
            break;
        }
        if self.identity.is_some() {
            return Ok(());
        }
        // An already-running adapter may emit no banner. It is now safe to
        // identify it: the voltage probe has completed and no reply is pending.
        tx = Transaction::default();
        let bytes = Command::Fixed(Fixed::Identify).encode(self.settings.cr)?;
        self.write_all(&bytes, deadline)?;
        self.read_prompt(&mut tx, "ATI", deadline)?;
        tx.check()?;
        self.observe_identity(&tx);
        if self.identity.is_none() {
            return Err(Error::malformed("synchronization identity probe failed"));
        }
        Ok(())
    }
    /// Bounded interruption/prompt synchronization, without a reset or request retry.
    pub fn resynchronize(&mut self, timeout: Option<Duration>, callback: impl FnOnce(Result<()>)) {
        let result = (|| {
            self.require_connection()?;
            let deadline = Deadline::new(timeout.unwrap_or(self.options.recovery_timeout))?;
            deadline.remaining()?;
            self.state = SessionState::Unsynchronized;
            self.decoder.clear();
            self.synchronize_if_needed(deadline)
        })();
        if result.is_err() && self.transport.is_some() {
            self.state = SessionState::Unusable;
        }
        callback(result);
    }
    fn recover_error(&mut self, error: &mut Error) {
        if self.transport.is_none() {
            return;
        }
        self.decoder.clear();
        self.literal_value_line = false;
        self.state = SessionState::Unsynchronized;
        let result = Deadline::new(self.options.recovery_timeout)
            .and_then(|d| self.synchronize_if_needed(d));
        if result.is_ok() {
            error.recovery = Recovery::Synchronized;
        } else {
            self.state = SessionState::Unusable;
            error.recovery = Recovery::ReconnectRequired;
        }
    }
    fn invalidate(&mut self) {
        let cr = self.settings.cr;
        let lf = self.settings.lf;
        let baud = self.settings.serial_baud;
        self.settings = Settings {
            cr,
            lf,
            serial_baud: baud,
            ..Settings::default()
        };
        self.last = None;
        self.flow_header = false;
        self.flow_data = false;
        self.user_formats = [None; 2];
    }
    fn apply_defaults(&mut self, reset: bool) -> Result<()> {
        self.invalidate();
        self.baud_timeout = 0x0F;
        if reset {
            for (index, id) in [0x2C, 0x2E].into_iter().enumerate() {
                self.user_formats[index] = match self.parameters.get(&id) {
                    Some((Some(value), Some(true))) => Some(value & 7),
                    Some((_, Some(false))) => Some(0),
                    _ => None,
                };
            }
            for (id, field) in [(0x0D, &mut self.settings.cr), (0x0A, &mut self.settings.lf)] {
                if let Some((value, Some(enabled))) = self.parameters.get(&id) {
                    *field = if *enabled {
                        value.unwrap_or(if id == 0x0D { b'\r' } else { b'\n' })
                    } else if id == 0x0D {
                        b'\r'
                    } else {
                        b'\n'
                    };
                }
            }
            self.decoder = Decoder::new(
                self.settings.cr,
                self.settings.lf,
                self.options.max_line_bytes,
            )?;
            self.framing = (self.settings.cr, self.settings.lf);
        }
        Ok(())
    }
    fn pp_baud(&self) -> Option<u32> {
        self.parameters.get(&0x0C).and_then(|(value, enabled)| {
            if *enabled == Some(true) {
                value.map(|v| {
                    if v == 0 {
                        9600
                    } else {
                        4_000_000 / u32::from(v)
                    }
                })
            } else {
                None
            }
        })
    }
    fn update(&mut self, command: &Command, response: &Response) -> Result<()> {
        match command {
            Command::Toggle(t, on) => match t {
                Toggle::Echo => self.settings.echo = Some(*on),
                Toggle::Linefeeds => self.settings.linefeeds = Some(*on),
                Toggle::Headers => self.settings.headers = Some(*on),
                Toggle::Spaces => self.settings.spaces = Some(*on),
                Toggle::Dlc => self.settings.dlc = Some(*on),
                Toggle::CanFormatting => self.settings.can_formatting = Some(*on),
                Toggle::Responses => self.settings.responses = Some(*on),
                Toggle::VariableDlc => self.settings.variable_dlc = Some(*on),
                _ => {}
            },
            Command::SetProtocol {
                protocol,
                automatic,
                ..
            } => {
                self.settings.protocol = Some(*protocol);
                self.settings.automatic = Some(*automatic || *protocol == Protocol::Automatic);
                self.settings.active_protocol = Some(false);
            }
            Command::EraseProtocol => {
                self.settings.protocol = Some(Protocol::Automatic);
                self.settings.automatic = Some(true);
                self.settings.active_protocol = Some(false);
            }
            Command::CanExtendedAddress(v) => self.settings.extended_address = Some(*v),
            Command::Byte(ByteSetting::TesterAddress, v) => self.settings.tester_address = Some(*v),
            Command::Byte(ByteSetting::BaudTimeout, v) => self.baud_timeout = *v,
            Command::Timer(v) => self.settings.timer_multiplier = Some(multiplier(*v)),
            Command::J1939Timer(v) => {
                self.settings.j1939_timer_multiplier = Some(multiplier(*v));
                if self.firmware == Firmware::V21 {
                    self.settings.timer_multiplier = Some(multiplier(*v));
                }
            }
            Command::Fixed(Fixed::AllowLong) => self.settings.long_messages = Some(true),
            Command::Fixed(Fixed::NormalLength) => self.settings.long_messages = Some(false),
            Command::Fixed(Fixed::Defaults) => self.apply_defaults(false)?,
            Command::RawAt(_) => self.invalidate(),
            Command::FlowHeader(_) => self.flow_header = true,
            Command::FlowData(_) => self.flow_data = true,
            Command::ProtocolB { options, .. } => self.user_formats[0] = Some(options & 7),
            Command::Vehicle { .. }
            | Command::Fixed(Fixed::BypassInit | Fixed::FastInit | Fixed::SlowInit) => {
                self.settings.active_protocol = Some(true)
            }
            Command::Fixed(Fixed::CloseProtocol) => self.settings.active_protocol = Some(false),
            Command::Parameter { target, action } => match target {
                ParameterTarget::One(id) => {
                    let entry = self.parameters.entry(*id).or_insert((None, None));
                    match action {
                        ParameterAction::Set(v) => entry.0 = Some(*v),
                        ParameterAction::Enable => entry.1 = Some(true),
                        ParameterAction::Disable => entry.1 = Some(false),
                    }
                }
                ParameterTarget::All => {
                    for id in 0..=0x2F {
                        self.parameters.entry(id).or_insert((None, None)).1 =
                            Some(*action == ParameterAction::Enable);
                    }
                }
            },
            _ => {}
        }
        match &response.value {
            ResponseValue::Parameters(values) => {
                for value in values {
                    self.parameters
                        .insert(value.id, (Some(value.value), Some(value.enabled)));
                }
            }
            ResponseValue::Firmware { version, identity } => {
                self.firmware = self.firmware_override.unwrap_or(*version);
                self.identity = Some(identity.clone());
            }
            ResponseValue::Protocol {
                protocol,
                automatic,
            } => {
                self.settings.protocol = Some(*protocol);
                self.settings.automatic = Some(*automatic);
            }
            _ => {}
        }
        Ok(())
    }

    fn baud_handshake(
        &mut self,
        command: &Command,
        echo: &str,
        deadline: Deadline,
        tx: &mut Transaction,
    ) -> Result<Response> {
        let Command::Byte(ByteSetting::BaudDivisor, divisor) = command else {
            return Err(Error::invalid("not BRD"));
        };
        let old = self
            .transport_mut()?
            .baud_rate()
            .ok_or_else(|| Error::new(ErrorKind::UnsupportedTransport, "not serial"))?;
        let new = 4_000_000 / u32::from(*divisor);
        loop {
            if let Some(event) = self.read_event(tx, deadline)? {
                match event {
                    FrameEvent::Line(s) => {
                        tx.add(s, echo);
                        tx.check()?;
                        if tx.records.last() == Some(&Record::Ack) {
                            break;
                        }
                    }
                    FrameEvent::Prompt => {
                        return Err(Error::malformed("BRD missing intermediate acknowledgment"));
                    }
                }
            }
        }
        self.transport_mut()?
            .set_baud_rate(new)
            .map_err(Error::io)?;
        self.settings.serial_baud = Some(new);
        let handshake = (|| {
            let units = if self.baud_timeout == 0 {
                256
            } else {
                u64::from(self.baud_timeout)
            };
            let wait = Duration::from_millis(units * 5);
            let handshake_deadline =
                Deadline::new(deadline.remaining()?.min(wait.saturating_mul(3)))?;
            let identity;
            loop {
                if let Some(event) = self.read_event(tx, handshake_deadline)? {
                    match event {
                        FrameEvent::Line(s) => {
                            if s.starts_with("ELM327 ") {
                                identity = s.clone();
                                tx.add(s, "");
                                break;
                            }
                            tx.add(s, "");
                            tx.check()?;
                        }
                        FrameEvent::Prompt => {
                            return Err(Error::malformed("BRD reverted before identity"));
                        }
                    }
                }
            }
            if self
                .identity
                .as_ref()
                .is_some_and(|expected| expected != &identity)
            {
                return Err(Error::malformed("new-baud identity mismatch"));
            }
            let ack_deadline = Deadline::new(deadline.remaining()?.min(wait))?;
            self.write_all(&[self.settings.cr], ack_deadline)?;
            let before = tx.records.len();
            self.read_prompt(tx, "", ack_deadline)?;
            tx.check()?;
            if !tx.records[before..].contains(&Record::Ack) {
                return Err(Error::malformed("BRD missing final acknowledgment"));
            }
            self.observe_identity(tx);
            self.remember_baud(new);
            Ok(Response {
                value: ResponseValue::Baud(new),
                transaction: tx.clone(),
            })
        })();
        if handshake.is_err() {
            self.transport_mut()?
                .set_baud_rate(old)
                .map_err(Error::io)?;
            self.settings.serial_baud = Some(old);
            self.remember_baud(old);
            self.state = SessionState::Unsynchronized;
        }
        handshake
    }

    /// Wake without automatically reinitializing. The wake character is not an ECU request.
    pub fn wake(&mut self, timeout: Option<Duration>, callback: impl FnOnce(Result<()>)) {
        let result = (|| {
            if self.transport.is_none() {
                return Err(Error::new(ErrorKind::Disconnected, "connect first"));
            }
            if self.state != SessionState::Asleep {
                return Err(Error::invalid("session is not asleep"));
            }
            let deadline = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))?;
            deadline.remaining()?;
            // LP enters standby about one second after its ACK; allow this transition.
            let delay = self
                .standby_enter_at
                .map(|at| at.saturating_duration_since(Instant::now()))
                .unwrap_or_default();
            if deadline.remaining()? <= delay {
                return Err(Error::new(
                    ErrorKind::Timeout,
                    "wake deadline must allow standby entry",
                ));
            }
            std::thread::sleep(delay);
            self.standby_enter_at = None;
            self.write_all(b" ", deadline)?;
            self.state = SessionState::Unsynchronized;
            self.synchronize_if_needed(deadline)
        })();
        if result.is_err() && self.state == SessionState::Unsynchronized {
            self.state = SessionState::Unusable;
        }
        callback(result);
    }

    pub fn monitor(
        &mut self,
        mode: Monitor,
        timeout: Option<Duration>,
        mut event: impl FnMut(&Record) -> MonitorControl,
        complete: impl FnOnce(Result<MonitorSummary>),
    ) {
        let mut summary = MonitorSummary::default();
        let result = (|| {
            self.require_connection()?;
            let command = Command::Monitor(mode);
            let bytes = command.encode(self.settings.cr)?;
            if matches!(
                &command,
                Command::Monitor(Monitor::Dm1 | Monitor::Pgn { .. })
            ) {
                let p = self.settings.protocol.ok_or_else(|| {
                    Error::new(ErrorKind::UnknownCapability, "select J1939 protocol first")
                })?;
                if p != Protocol::J1939
                    && !(matches!(p, Protocol::User1 | Protocol::User2)
                        && self.user_format(p) == Some(2))
                {
                    return Err(Error::invalid("DM1/MP require J1939 formatting"));
                }
            }
            let deadline = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))?;
            deadline.remaining()?;
            self.synchronize_if_needed(deadline)?;
            let echo = if self.settings.echo == Some(false) {
                String::new()
            } else {
                String::from_utf8_lossy(&bytes[..bytes.len() - 1]).into_owned()
            };
            self.write_all(&bytes, deadline)?;
            self.settings.active_protocol = Some(false);
            loop {
                if let Some(frame) = self.read_event(&mut summary.transaction, deadline)? {
                    match frame {
                        FrameEvent::Prompt => {
                            self.state = SessionState::Ready;
                            summary.transaction.check()?;
                            break;
                        }
                        FrameEvent::Line(line) => {
                            let before = summary.transaction.records.len();
                            summary.transaction.add(line, &echo);
                            if before == summary.transaction.records.len() {
                                continue;
                            }
                            let record = summary.transaction.records.last().expect("new record");
                            summary.events += 1;
                            let stop = event(record) == MonitorControl::Stop;
                            if matches!(record, Record::Identity(_)) {
                                self.invalidate();
                                return Err(Error::new(
                                    ErrorKind::Reset,
                                    "unexpected startup during monitoring",
                                ));
                            }
                            if matches!(record,Record::Alert(s) if s.ends_with("LP ALERT")) {
                                self.state = SessionState::Asleep;
                                self.standby_enter_at =
                                    Instant::now().checked_add(Duration::from_secs(2));
                                return Err(Error::new(
                                    ErrorKind::UnusableSession,
                                    "bridge entering low power",
                                ));
                            }
                            if matches!(record, Record::Reset(_)) {
                                self.invalidate();
                            }
                            // Line annotations are streamed intact; terminal summary retains them.
                            if matches!(record, Record::Fault(_) | Record::Reset(_)) {
                                summary.transaction.check()?;
                            }
                            if stop {
                                summary.stopped_by_caller = true;
                                self.write_all(b" ", deadline)?;
                                self.read_prompt(&mut summary.transaction, &echo, deadline)?;
                                // Annotated frames were delivered and retained, not silently valid.
                                for r in &summary.transaction.records {
                                    if matches!(r, Record::Fault(_) | Record::Reset(_)) {
                                        summary.transaction.check()?;
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
            self.last = Some(command);
            Ok(())
        })();
        match result {
            Ok(()) => {
                if !self.options.capture_raw {
                    summary.transaction.raw.clear();
                }
                complete(Ok(summary));
            }
            Err(mut e) => {
                if e.partial.is_empty() {
                    e.partial = summary.transaction.raw;
                }
                if self.transport.is_some()
                    && self.state != SessionState::Ready
                    && self.state != SessionState::Unusable
                    && self.state != SessionState::Asleep
                {
                    self.recover_error(&mut e);
                }
                complete(Err(e));
            }
        }
    }

    /// Normalize H1/CAF1/D0/S1/R1 and disable extended addressing, request data,
    /// query DPN, then reassemble by sender. These documented preparatory AT
    /// transactions share one deadline and never reset/clear an ECU.
    pub fn request_obd(
        &mut self,
        bytes: Vec<u8>,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Vec<EcuMessage>>),
    ) {
        let result = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))
            .and_then(|deadline| self.obd_inner(bytes, deadline))
            .map(|batch| batch.messages);
        callback(result);
    }
    /// Like `request_obd`, retaining successful wire evidence for recording.
    /// ECU-specific negative responses remain individual message payloads.
    pub fn request_obd_evidence(
        &mut self,
        bytes: Vec<u8>,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<ObdEvidence>),
    ) {
        let result = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))
            .and_then(|deadline| self.obd_inner(bytes, deadline))
            .map(|batch| ObdEvidence {
                messages: batch.messages,
                raw: batch.raw,
                protocol: self.settings.protocol.expect("obd_inner verified protocol"),
            });
        callback(result);
    }
    fn obd_inner(&mut self, bytes: Vec<u8>, deadline: Deadline) -> Result<ObdBatch> {
        self.require_connection()?;
        deadline.remaining()?;
        Command::Vehicle {
            bytes: bytes.clone(),
            responses: None,
        }
        .encode(self.settings.cr)?;
        if self.settings.protocol.is_none() {
            self.execute_inner(
                Command::Fixed(Fixed::ProtocolNumber),
                Some(deadline.remaining()?),
            )?;
        }
        if self
            .settings
            .protocol
            .is_some_and(|p| matches!(p, Protocol::J1939 | Protocol::User1 | Protocol::User2))
        {
            return Err(Error::invalid(
                "high-level OBD requires standard OBD protocol",
            ));
        }
        for (toggle, value, current) in [
            (Toggle::Headers, true, self.settings.headers),
            (Toggle::CanFormatting, true, self.settings.can_formatting),
            (Toggle::Dlc, false, self.settings.dlc),
            (Toggle::Spaces, true, self.settings.spaces),
            (Toggle::Responses, true, self.settings.responses),
        ] {
            if current != Some(value) {
                self.execute_inner(Command::Toggle(toggle, value), Some(deadline.remaining()?))?;
            }
        }
        if self.settings.extended_address != Some(None) {
            self.execute_inner(
                Command::CanExtendedAddress(None),
                Some(deadline.remaining()?),
            )?;
        }
        let response = self.execute_inner(
            Command::Vehicle {
                bytes,
                responses: None,
            },
            Some(deadline.remaining()?),
        )?;
        self.execute_inner(
            Command::Fixed(Fixed::ProtocolNumber),
            Some(deadline.remaining()?),
        )
        .map_err(|mut error| {
            let mut raw = response.transaction.raw.clone();
            raw.extend(&error.partial);
            error.partial = raw;
            error
        })?;
        let protocol = self
            .settings
            .protocol
            .ok_or_else(|| Error::new(ErrorKind::UnknownCapability, "unknown active protocol"))?;
        let layout = match protocol {
            Protocol::Can11_500 | Protocol::Can11_250 => Layout::Can {
                extended_id: false,
                dlc: false,
                extended_address: None,
            },
            Protocol::Can29_500 | Protocol::Can29_250 => Layout::Can {
                extended_id: true,
                dlc: false,
                extended_address: None,
            },
            Protocol::J1850Pwm
            | Protocol::J1850Vpw
            | Protocol::Iso9141
            | Protocol::KwpSlow
            | Protocol::KwpFast => Layout::Legacy(protocol),
            _ => {
                return Err(Error::invalid(
                    "high-level OBD requires standard OBD protocol",
                ));
            }
        };
        let messages = obd::reassemble(&response.transaction, layout)?;
        let pending: Vec<_> = messages
            .iter()
            .filter(|m| m.bytes.len() == 3 && m.bytes[0] == 0x7F && m.bytes[2] == 0x78)
            .collect();
        let messages: Vec<_> = messages
            .iter()
            .filter(|m| !(m.bytes.len() == 3 && m.bytes[0] == 0x7F && m.bytes[2] == 0x78))
            .cloned()
            .collect();
        if pending
            .iter()
            .any(|p| !messages.iter().any(|m| m.sender == p.sender))
        {
            let mut error = Error::new(
                ErrorKind::IncompleteMessage,
                "pending ECU has no final response",
            );
            error.partial = response.transaction.raw;
            return Err(error);
        }
        Ok(ObdBatch {
            messages,
            raw: response.transaction.raw,
        })
    }
    pub fn read_pid(
        &mut self,
        pid: obd::Pid,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Vec<obd::Measurement>>),
    ) {
        let result = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))
            .and_then(|d| self.obd_inner(vec![1, pid as u8], d))
            .and_then(|batch| batch.decode(|ms| ms.iter().map(|m| pid.decode(m)).collect()));
        callback(result);
    }
    pub fn supported_pids(
        &mut self,
        base: u8,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Vec<obd::SupportedPids>>),
    ) {
        let result = (|| {
            if base % 32 != 0 || base > 0xE0 {
                return Err(Error::invalid("invalid PID support block"));
            }
            let d = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))?;
            let batch = self.obd_inner(vec![1, base], d)?;
            batch.decode(|ms| {
                ms.iter()
                    .map(|m| obd::SupportedPids::decode(m, base))
                    .collect()
            })
        })();
        callback(result);
    }
    /// Discover subsequent support blocks while each ECU advertises them.
    pub fn discover_pids(
        &mut self,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Vec<obd::SupportedPids>>),
    ) {
        let result = (|| {
            let d = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))?;
            let mut out = Vec::new();
            let mut wanted: Option<Vec<obd::Sender>> = None;
            for base in (0..=0xE0).step_by(32) {
                let batch = self.obd_inner(vec![1, base], d)?;
                let mut next = Vec::new();
                batch.decode(|messages| {
                    if wanted.as_ref().is_some_and(|senders| {
                        senders
                            .iter()
                            .any(|s| !messages.iter().any(|m| m.sender == *s))
                    }) {
                        return Err(Error::new(
                            ErrorKind::IncompleteMessage,
                            "ECU advertising a support block did not respond",
                        ));
                    }
                    for m in messages {
                        let block = obd::SupportedPids::decode(m, base)?;
                        if wanted
                            .as_ref()
                            .is_none_or(|senders| senders.contains(&block.sender))
                        {
                            if block.has_next_block() {
                                next.push(block.sender);
                            }
                            out.push(block);
                        }
                    }
                    Ok(())
                })?;
                if next.is_empty() {
                    break;
                }
                wanted = Some(next);
            }
            Ok(out)
        })();
        callback(result);
    }
    pub fn read_vin(
        &mut self,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Vec<obd::Vin>>),
    ) {
        let result = (|| {
            let d = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))?;
            let batch = self.obd_inner(vec![9, 2], d)?;
            let can = self.settings.protocol.is_some_and(Protocol::is_can);
            batch.decode(|ms| obd::decode_vins(ms, can))
        })();
        callback(result);
    }
    pub fn read_dtcs(
        &mut self,
        kind: obd::DtcKind,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Vec<obd::TroubleCodes>>),
    ) {
        let result = (|| {
            let d = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))?;
            let batch = self.obd_inner(vec![kind.service()], d)?;
            let can = self.settings.protocol.is_some_and(Protocol::is_can);
            batch.decode(|ms| {
                let mut grouped: BTreeMap<obd::Sender, obd::TroubleCodes> = BTreeMap::new();
                for message in ms {
                    let decoded = obd::decode_dtcs(message, kind, can)?;
                    grouped
                        .entry(decoded.sender)
                        .or_insert_with(|| obd::TroubleCodes {
                            sender: decoded.sender,
                            kind,
                            codes: Vec::new(),
                        })
                        .codes
                        .extend(decoded.codes);
                }
                Ok(grouped.into_values().collect())
            })
        })();
        callback(result);
    }
    /// Explicit destructive ECU operation: clears stored diagnostics/freeze frames,
    /// not permanent DTCs. The caller must decide/confirm whether to perform it.
    pub fn clear_dtcs(
        &mut self,
        timeout: Option<Duration>,
        callback: impl FnOnce(Result<Vec<EcuMessage>>),
    ) {
        let result = (|| {
            let d = Deadline::new(timeout.unwrap_or(self.options.operation_timeout))?;
            let batch = self.obd_inner(vec![4], d)?;
            batch.decode(|ms| {
                for m in ms {
                    obd::positive(m, 4)?;
                }
                Ok(ms.to_vec())
            })
        })();
        callback(result);
    }
}

struct ObdBatch {
    messages: Vec<EcuMessage>,
    raw: Vec<u8>,
}

impl Client {
    fn user_format(&self, protocol: Protocol) -> Option<u8> {
        match protocol {
            Protocol::User1 => self.user_formats[0],
            Protocol::User2 => self.user_formats[1],
            _ => None,
        }
    }
    fn remember_baud(&mut self, baud: u32) {
        if let TransportConfig::Serial(config) = &mut self.config {
            config.baud_rate = baud;
        }
    }
}
impl ObdBatch {
    fn decode<T>(self, decode: impl FnOnce(&[EcuMessage]) -> Result<T>) -> Result<T> {
        decode(&self.messages).map_err(|mut error| {
            error.partial = self.raw;
            error
        })
    }
}

#[derive(Clone, Copy)]
struct Deadline(Instant);
impl Deadline {
    fn new(duration: Duration) -> Result<Self> {
        Instant::now()
            .checked_add(duration)
            .map(Self)
            .ok_or_else(|| Error::invalid("timeout too large"))
    }
    fn remaining(self) -> Result<Duration> {
        let d = self.0.saturating_duration_since(Instant::now());
        if d.is_zero() {
            Err(Error::new(ErrorKind::Timeout, "operation deadline expired"))
        } else {
            Ok(d)
        }
    }
}
fn is_special_raw(s: &str) -> bool {
    let s = s.to_ascii_uppercase().replace(' ', "");
    [
        "ATBRD", "ATLP", "ATMA", "ATMR", "ATMT", "ATDM1", "ATMP", "ATZ", "ATWS",
    ]
    .iter()
    .any(|prefix| s.starts_with(prefix))
}
