use kittx::{
    command::{Command, Fixed},
    response::ResponseValue,
    *,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

#[test]
fn real_tcp_fragmentation_callbacks_and_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream.write_all(b"ELM327 v2.0\r>").unwrap();
            let mut command = [0; 5];
            stream.read_exact(&mut command).unwrap();
            assert_eq!(&command, b"ATRV\r");
            for part in [b"AT".as_slice(), b"RV\r12.", b"\x006V\r\n>"] {
                stream.write_all(part).unwrap();
            }
            let mut end = [0];
            assert_eq!(stream.read(&mut end).unwrap(), 0);
        }
    });
    let mut c = Client::new(TransportConfig::tcp("localhost", port));
    for _ in 0..2 {
        c.connect(None, Result::unwrap);
        let thread = thread::current().id();
        c.read_voltage(None, |r| {
            let r = r.unwrap();
            assert_eq!(r.value, ResponseValue::Voltage(12.6));
            assert!(r.transaction.raw.contains(&0));
            assert_eq!(thread::current().id(), thread);
        });
        assert_eq!(c.firmware(), Firmware::V20);
        c.disconnect(None, Result::unwrap);
    }
    server.join().unwrap();
}
#[test]
fn real_tcp_optional_initialization_and_coalesced_lines() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        s.write_all(b">").unwrap();
        for (command, reply) in [
            ("ATI\r", "ELM327 v2.1\r>"),
            ("ATE0\r", "OK\r>"),
            (
                "0100\r",
                "SEARCHING...\r41 00 BE 1F B8 10\r41 00 80 10 80 00\r>",
            ),
        ] {
            let mut buffer = vec![0; command.len()];
            s.read_exact(&mut buffer).unwrap();
            assert_eq!(buffer, command.as_bytes());
            s.write_all(reply.as_bytes()).unwrap();
        }
    });
    let mut c = Client::with_initialization(
        TransportConfig::tcp("127.0.0.1", port),
        Initialization::new(vec![
            Command::Fixed(Fixed::Identify),
            Command::Toggle(kittx::command::Toggle::Echo, false),
        ]),
    );
    c.connect(None, Result::unwrap);
    assert_eq!(c.settings().echo, Some(false));
    c.send_vehicle(vec![1, 0], None, None, |r| {
        assert_eq!(r.unwrap().transaction.lines.len(), 3)
    });
    c.disconnect(None, Result::unwrap);
    server.join().unwrap();
}
#[test]
fn real_tcp_deadline_partial_response_and_interrupt_recovery() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        s.write_all(b">").unwrap();
        let mut bytes = [0; 5];
        s.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"ATRV\r");
        s.write_all(b"12.").unwrap();
        let mut interrupt = [0];
        s.read_exact(&mut interrupt).unwrap();
        assert_eq!(&interrupt, b" ");
        s.write_all(b"STOPPED\r>").unwrap();
        s.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"ATRV\r");
        s.write_all(b"12.8V\r>").unwrap();
    });
    let options = ClientOptions {
        synchronization_probe: Duration::from_millis(5),
        ..ClientOptions::default()
    };
    let mut c = Client::with_options(TransportConfig::tcp("127.0.0.1", port), options).unwrap();
    c.connect(None, Result::unwrap);
    c.read_voltage(Some(Duration::from_millis(100)), |r| {
        let error = r.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Timeout);
        assert_eq!(error.partial, b"12.");
        assert_eq!(error.recovery, Recovery::Synchronized);
    });
    c.read_voltage(None, |r| {
        assert_eq!(r.unwrap().value, ResponseValue::Voltage(12.8))
    });
    c.disconnect(None, Result::unwrap);
    server.join().unwrap();
}
#[test]
fn real_tcp_eof_marks_session_unusable() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        s.write_all(b">").unwrap();
        let mut command = [0; 5];
        s.read_exact(&mut command).unwrap();
        s.write_all(b"12.").unwrap();
    });
    let mut c = Client::new(TransportConfig::tcp("127.0.0.1", port));
    c.connect(None, Result::unwrap);
    c.read_voltage(None, |r| {
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Io(std::io::ErrorKind::UnexpectedEof));
        assert_eq!(e.recovery, Recovery::ReconnectRequired);
    });
    assert_eq!(c.state(), SessionState::Unusable);
    c.disconnect(None, Result::unwrap);
    server.join().unwrap();
}
#[test]
fn real_tcp_connection_refused_and_invalid_host() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut c = Client::new(TransportConfig::tcp("127.0.0.1", port));
    c.connect(Some(Duration::from_millis(200)), |r| {
        assert!(matches!(r.unwrap_err().kind, ErrorKind::Io(_)))
    });
    assert_eq!(c.state(), SessionState::Disconnected);
    let mut c = Client::new(TransportConfig::tcp("", 0));
    c.connect(None, |r| {
        assert_eq!(r.unwrap_err().kind, ErrorKind::InvalidArgument)
    });
}
