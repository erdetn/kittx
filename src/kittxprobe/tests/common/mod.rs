//! Local ELM327 transcript server. Tests never use a real adapter or remote host.
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct ScriptServer {
    pub port: u16,
    worker: JoinHandle<Vec<String>>,
}

impl ScriptServer {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream.write_all(b">").unwrap();
            let mut requests = Vec::new();
            let mut command = Vec::new();
            let mut byte = [0];
            loop {
                match stream.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) if byte[0] != b'\r' => command.push(byte[0]),
                    Ok(_) => {
                        let text = String::from_utf8_lossy(&command).trim().to_uppercase();
                        let response = match text.as_str() {
                            "ATI" => "ELM327 v2.1\r>".into(),
                            "ATDPN" => "6\r>".into(),
                            "ATRV" => "12.4V\r>".into(),
                            "0902" => "7E8 10 14 49 02 01 57 44 42\r7E8 21 31 32 33 34 35 36 37\r7E8 22 38 39 30 31 32 33 34\r>".into(),
                            "010C" => "7E8 04 41 0C 1A F4 00 00 00\r>".into(),
                            "0161" => "7E8 03 41 61 80 00 00 00 00\r>".into(),
                            "03" => "7E8 04 43 01 13 37 00 00 00\r>".into(),
                            "07" => "7E8 02 47 00 00 00 00 00 00\r>".into(),
                            "0A" => "7E8 04 4A 01 04 56 00 00 00\r>".into(),
                            s if s.starts_with("AT") => "OK\r>".into(),
                            s if s.len()==4 && s.starts_with("01") && u8::from_str_radix(&s[2..],16).is_ok_and(|pid| pid%32==0) => {
                                format!("7E8 06 41 {} FF FF FF FF 00\r>",&s[2..])
                            }
                            _ => "NO DATA\r>".into(),
                        };
                        requests.push(text);
                        stream.write_all(response.as_bytes()).unwrap();
                        command.clear();
                    }
                    Err(error) => panic!("Local transcript read failed: {error}"),
                }
            }
            requests
        });
        Self { port, worker }
    }
    pub fn finish(self) -> Vec<String> {
        self.worker.join().unwrap()
    }
}
