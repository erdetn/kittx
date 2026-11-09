use super::*;
use crate::{
    Client, Command, Error, ErrorKind, Protocol, Recovery, ResponseValue, SessionState,
    command::{CanId, Header},
    obd::{self, EcuMessage, Sender},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// Synchronous acquisition boundary; implementations must honor the supplied deadline.
/// Errors in individual ECU replies belong in `Observation`, not a lost successful peer.
pub trait DataSource {
    fn connect(&mut self, timeout: Duration) -> DataResult<SourceInfo>;
    fn read(&mut self, request: &ReadRequest, timeout: Duration) -> Observation;
    fn disconnect(&mut self, timeout: Duration) -> DataResult<()>;
}
/// Owns a communication client exclusively for the duration of a recording.
pub struct ClientSource {
    client: Client,
}
impl ClientSource {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
    pub fn client(&self) -> &Client {
        &self.client
    }
    fn execute(&mut self, command: Command, timeout: Duration) -> crate::Result<crate::Response> {
        let mut out = Err(Error::new(
            ErrorKind::InvalidArgument,
            "callback not invoked",
        ));
        self.client.execute(command, Some(timeout), |r| out = r);
        out
    }
    fn obd(&mut self, bytes: Vec<u8>, timeout: Duration) -> crate::Result<crate::ObdEvidence> {
        let mut out = Err(Error::new(
            ErrorKind::InvalidArgument,
            "callback not invoked",
        ));
        self.client
            .request_obd_evidence(bytes, Some(timeout), |r| out = r);
        out
    }
    fn profile(&mut self, profile: &DiagnosticProfile, timeout: Duration) -> Observation {
        if !self.client.options().capture_raw {
            return failure(Error::new(
                ErrorKind::InvalidArgument,
                "raw profiles require raw capture enabled",
            ));
        }
        let began = Instant::now();
        let remaining = || {
            timeout
                .checked_sub(began.elapsed())
                .filter(|d| !d.is_zero())
                .ok_or_else(|| Error::new(ErrorKind::Timeout, "profile deadline expired"))
        };
        let protocol = self.client.settings().protocol;
        let is_can = matches!(
            protocol,
            Some(
                Protocol::Can11_500
                    | Protocol::Can11_250
                    | Protocol::Can29_500
                    | Protocol::Can29_250
            )
        );
        if (profile.header.is_some() || profile.receive_filter.is_some()) && !is_can {
            return failure(Error::new(
                ErrorKind::InvalidArgument,
                "targeted raw profiles require a known standard CAN protocol",
            ));
        }
        let changed = profile.header.is_some() || profile.receive_filter.is_some();
        let result = (|| {
            if let Some(h) = profile.header {
                self.execute(Command::Header(h), remaining()?)?;
            }
            if profile.receive_filter.is_some() {
                self.execute(
                    Command::CanReceive(profile.receive_filter.clone()),
                    remaining()?,
                )?;
            }
            self.execute(
                Command::Vehicle {
                    bytes: profile.request.clone(),
                    responses: None,
                },
                remaining()?,
            )
        })();
        // Restore functional OBD addressing before any subsequent standard acquisition.
        let restored = if changed {
            (|| {
                self.execute(Command::CanReceive(None), remaining()?)?;
                let header = if matches!(protocol, Some(Protocol::Can29_250 | Protocol::Can29_500))
                {
                    CanId::Extended(0x18DB33F1)
                } else {
                    CanId::Standard(0x7DF)
                };
                self.execute(Command::Header(Header::Can(header)), remaining()?)?;
                Ok::<_, Error>(())
            })()
        } else {
            Ok(())
        };
        let mut observation = match result {
            Ok(r) => Observation {
                protocol,
                raw_response: self
                    .client
                    .options()
                    .capture_raw
                    .then(|| r.transaction.raw.clone()),
                responses: vec![EcuObservation {
                    sender: profile.target.unwrap_or(Sender::Unknown),
                    value: ObservationValue::Raw(r.transaction.raw),
                }],
                ..Observation::default()
            },
            Err(e) => failure(e),
        };
        if let Err(e) = restored {
            observation.error = Some(format!("profile addressing restoration failed: {e}"));
            observation.disconnected = true;
        }
        observation.attempted_request = Command::Vehicle {
            bytes: profile.request.clone(),
            responses: None,
        }
        .encode(self.client.settings().cr)
        .ok();
        observation.context = Some(format!(
            "raw profile response; namespace={}; presentation={:?}; request is intended bytes, not a write acknowledgment",
            profile.namespace,
            self.client.settings()
        ));
        observation
    }
}
impl DataSource for ClientSource {
    fn connect(&mut self, timeout: Duration) -> DataResult<SourceInfo> {
        let start = Instant::now();
        let mut result = Ok(());
        self.client.connect(Some(timeout), |r| result = r);
        result?;
        let remaining = timeout
            .checked_sub(start.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| Error::new(ErrorKind::Timeout, "connect/identity deadline expired"))?;
        self.execute(Command::Fixed(crate::command::Fixed::Identify), remaining)?;
        Ok(SourceInfo {
            transport: format!("{:?}", self.client.config()),
            adapter: self.client.identity().unwrap_or("unknown").into(),
        })
    }
    fn read(&mut self, request: &ReadRequest, timeout: Duration) -> Observation {
        if let ReadRequest::Diagnostic(p) = request {
            return self.profile(p, timeout);
        }
        if matches!(request, ReadRequest::Signal(Signal::AdapterVoltage)) {
            let mut result = Err(Error::new(
                ErrorKind::InvalidArgument,
                "callback not invoked",
            ));
            self.client.read_voltage(Some(timeout), |r| result = r);
            let mut observation = match result {
                Ok(r) => match r.value {
                    ResponseValue::Voltage(v) => Observation {
                        responses: vec![EcuObservation {
                            sender: Sender::Unknown,
                            value: ObservationValue::Number(v),
                        }],
                        raw_response: self
                            .client
                            .options()
                            .capture_raw
                            .then_some(r.transaction.raw),
                        ..Observation::default()
                    },
                    _ => failure(Error::new(
                        ErrorKind::MalformedResponse,
                        "voltage value missing",
                    )),
                },
                Err(e) => failure(e),
            };
            observation.attempted_request = Command::Fixed(crate::command::Fixed::Voltage)
                .encode(self.client.settings().cr)
                .ok();
            observation.context=Some("ATRV adapter supply voltage; response bytes include bridge framing; intended request is not a write acknowledgment".into());
            return observation;
        }
        let bytes = match request {
            ReadRequest::Vin => vec![9, 2],
            ReadRequest::Dtcs(k) => vec![k.service()],
            ReadRequest::Signal(s) => vec![1, s.pid().expect("voltage handled")],
            ReadRequest::Supported(b) => vec![1, *b],
            _ => unreachable!(),
        };
        let attempted = Command::Vehicle {
            bytes: bytes.clone(),
            responses: None,
        }
        .encode(self.client.settings().cr)
        .ok();
        let mut observation = match self.obd(bytes, timeout) {
            Err(e) => {
                let mut f = failure(e);
                f.protocol = self.client.settings().protocol;
                f.disconnected |= matches!(
                    self.client.state(),
                    SessionState::Unusable | SessionState::Disconnected
                );
                f
            }
            Ok(batch) => {
                let mut groups: BTreeMap<Sender, Vec<EcuMessage>> = BTreeMap::new();
                for message in batch.messages {
                    groups.entry(message.sender).or_default().push(message);
                }
                let responses = groups
                    .into_iter()
                    .map(|(sender, messages)| {
                        let decoded: crate::Result<ObservationValue> = (|| match request {
                            ReadRequest::Vin => Ok(ObservationValue::Vin(
                                obd::decode_vins(&messages, batch.protocol.is_can())?[0]
                                    .value
                                    .clone(),
                            )),
                            ReadRequest::Dtcs(kind) => {
                                let mut codes = Vec::new();
                                for message in &messages {
                                    for code in
                                        obd::decode_dtcs(message, *kind, batch.protocol.is_can())?
                                            .codes
                                    {
                                        let prefix = match code.system {
                                            obd::DtcSystem::Powertrain => 0,
                                            obd::DtcSystem::Chassis => 1,
                                            obd::DtcSystem::Body => 2,
                                            obd::DtcSystem::Network => 3,
                                        };
                                        codes.push(DiagnosticCode {
                                            text: code.to_string(),
                                            bytes: ((prefix << 14) | code.number)
                                                .to_be_bytes()
                                                .to_vec(),
                                            status: None,
                                        });
                                    }
                                }
                                Ok(ObservationValue::Dtcs(codes))
                            }
                            ReadRequest::Signal(Signal::Pid(pid)) => {
                                if messages.len() != 1 {
                                    return Err(Error::new(
                                        ErrorKind::AmbiguousSender,
                                        "multiple live replies from one ECU",
                                    ));
                                }
                                Ok(ObservationValue::Number(pid.decode(&messages[0])?.value))
                            }
                            ReadRequest::Signal(Signal::RawPid(pid)) => {
                                if messages.len() != 1 {
                                    return Err(Error::new(
                                        ErrorKind::AmbiguousSender,
                                        "multiple raw PID replies from one ECU",
                                    ));
                                }
                                obd::positive(&messages[0], 1)?;
                                if messages[0].bytes.get(1) != Some(pid) {
                                    return Err(Error::new(
                                        ErrorKind::MalformedResponse,
                                        "raw PID mismatch",
                                    ));
                                }
                                Ok(ObservationValue::Raw(messages[0].bytes.clone()))
                            }
                            ReadRequest::Supported(base) => {
                                if messages.len() != 1 {
                                    return Err(Error::new(
                                        ErrorKind::AmbiguousSender,
                                        "multiple capability replies from one ECU",
                                    ));
                                }
                                Ok(ObservationValue::Supported(
                                    obd::SupportedPids::decode(&messages[0], *base)?.bitmap,
                                ))
                            }
                            _ => unreachable!(),
                        })();
                        EcuObservation {
                            sender,
                            value: decoded.unwrap_or_else(|e| ObservationValue::Failure {
                                kind: format!("{:?}", e.kind),
                                message: e.to_string(),
                            }),
                        }
                    })
                    .collect();
                Observation {
                    protocol: Some(batch.protocol),
                    responses,
                    raw_response: self.client.options().capture_raw.then_some(batch.raw),
                    ..Observation::default()
                }
            }
        };
        observation.attempted_request = attempted;
        observation.context=Some("standard OBD; H1 CAF1 D0 S1 R1 CEA disabled; ECU response only; error partial can include protocol query; intended request is not a write acknowledgment".into());
        observation
    }
    fn disconnect(&mut self, timeout: Duration) -> DataResult<()> {
        let mut r = Ok(());
        self.client.disconnect(Some(timeout), |out| r = out);
        r.map_err(Into::into)
    }
}
fn failure(e: Error) -> Observation {
    Observation {
        error: Some(e.to_string()),
        partial: true,
        raw_response: (!e.partial.is_empty()).then_some(e.partial),
        recovery: Some(format!("{:?}", e.recovery)),
        disconnected: e.recovery == Recovery::ReconnectRequired
            || matches!(e.kind, ErrorKind::Disconnected | ErrorKind::UnusableSession),
        ..Observation::default()
    }
}
