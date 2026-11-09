#![allow(dead_code)]
use kittx::{
    Client, ClientOptions, Initialization, Result, TransportConfig,
    response::{Record, Transaction},
    transport::{Connector, Transport},
};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub enum Reply {
    Bytes(Vec<u8>),
    Timeout,
    Failure(io::ErrorKind),
}
#[derive(Clone)]
pub struct Step {
    pub expected: Vec<u8>,
    pub reply: Reply,
}
pub fn step(command: &str, response: &str) -> Step {
    Step {
        expected: command.as_bytes().to_vec(),
        reply: Reply::Bytes(response.as_bytes().to_vec()),
    }
}
#[derive(Default)]
pub struct Trace {
    pub opens: usize,
    pub writes: Vec<u8>,
    pub baud_changes: Vec<u32>,
    pub timeouts: Vec<Duration>,
    pub drops: usize,
}
pub struct FakeConnector {
    pub steps: Vec<Step>,
    pub startup: Vec<u8>,
    pub trace: Arc<Mutex<Trace>>,
    pub baud: Option<u32>,
    pub max_write: usize,
    pub fail_open: bool,
}
impl Connector for FakeConnector {
    fn open(&mut self, _: &TransportConfig, _: Duration) -> Result<Box<dyn Transport>> {
        self.trace.lock().unwrap().opens += 1;
        if self.fail_open {
            return Err(kittx::Error::new(
                kittx::ErrorKind::Io(io::ErrorKind::ConnectionRefused),
                "scripted open failure",
            ));
        }
        Ok(Box::new(FakeTransport {
            steps: self.steps.clone().into(),
            input: self.startup.clone().into(),
            trace: self.trace.clone(),
            baud: self.baud,
            max_write: self.max_write,
            buffer: Vec::new(),
            fault: None,
        }))
    }
}
struct FakeTransport {
    steps: VecDeque<Step>,
    input: VecDeque<u8>,
    trace: Arc<Mutex<Trace>>,
    baud: Option<u32>,
    max_write: usize,
    buffer: Vec<u8>,
    fault: Option<Reply>,
}
impl Drop for FakeTransport {
    fn drop(&mut self) {
        self.trace
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .drops += 1;
    }
}
impl Transport for FakeTransport {
    fn read(&mut self, bytes: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.trace.lock().unwrap().timeouts.push(timeout);
        if let Some(v) = self.input.pop_front() {
            bytes[0] = v;
            return Ok(1);
        }
        match self.fault.take() {
            Some(Reply::Failure(k)) => Err(io::Error::new(k, "scripted read error")),
            _ => Err(io::Error::new(io::ErrorKind::TimedOut, "scripted timeout")),
        }
    }
    fn write(&mut self, bytes: &[u8], timeout: Duration) -> io::Result<usize> {
        self.trace.lock().unwrap().timeouts.push(timeout);
        let n = bytes.len().min(self.max_write);
        self.trace
            .lock()
            .unwrap()
            .writes
            .extend_from_slice(&bytes[..n]);
        self.buffer.extend_from_slice(&bytes[..n]);
        let expected = &self
            .steps
            .front()
            .expect("unexpected write without scripted step")
            .expected;
        assert!(
            expected.starts_with(&self.buffer),
            "expected {:?}, received {:?}",
            String::from_utf8_lossy(expected),
            String::from_utf8_lossy(&self.buffer)
        );
        if self.buffer.len() == expected.len() {
            let step = self.steps.pop_front().unwrap();
            self.buffer.clear();
            match step.reply {
                Reply::Bytes(b) => self.input.extend(b),
                r => self.fault = Some(r),
            }
        }
        Ok(n)
    }
    fn baud_rate(&self) -> Option<u32> {
        self.baud
    }
    fn set_baud_rate(&mut self, v: u32) -> io::Result<()> {
        if self.baud.is_none() {
            return Err(io::ErrorKind::Unsupported.into());
        }
        self.baud = Some(v);
        self.trace.lock().unwrap().baud_changes.push(v);
        Ok(())
    }
}
pub fn client(steps: Vec<Step>) -> (Client, Arc<Mutex<Trace>>) {
    client_with(
        steps,
        b"ELM327 v2.1\r>".to_vec(),
        Initialization::default(),
        None,
    )
}
pub fn client_with(
    steps: Vec<Step>,
    startup: Vec<u8>,
    initialization: Initialization,
    baud: Option<u32>,
) -> (Client, Arc<Mutex<Trace>>) {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let connector = FakeConnector {
        steps,
        startup,
        trace: trace.clone(),
        baud,
        max_write: 2,
        fail_open: false,
    };
    let options = ClientOptions {
        recovery_timeout: Duration::from_millis(10),
        ..ClientOptions::default()
    };
    let c = Client::with_connector(
        baud.map(|rate| TransportConfig::Serial(kittx::SerialConfig::new("scripted-serial", rate)))
            .unwrap_or_else(|| TransportConfig::tcp("emulator", 35000)),
        options,
        initialization,
        Box::new(connector),
    )
    .unwrap();
    (c, trace)
}
pub fn connect(c: &mut Client) {
    let mut called = false;
    c.connect(None, |r| {
        r.unwrap();
        called = true;
    });
    assert!(called);
}
pub fn transaction(lines: &[&str]) -> Transaction {
    Transaction {
        raw: format!("{}\r>", lines.join("\r")).into_bytes(),
        lines: lines.iter().map(|s| s.to_string()).collect(),
        records: lines.iter().map(|s| Record::parse(s)).collect(),
    }
}
