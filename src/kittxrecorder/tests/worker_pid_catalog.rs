//! Verify the picker catalog against the real wire and SQLite path, without a car.
use kittx::{
    TransportConfig,
    datastore::{Query, RecordingReader},
};
use kittx_recorder::{
    model::{CaptureSettings, capture_catalog},
    recording::{Phase, RecordingJob},
};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

fn payload(pid: u8) -> Vec<u8> {
    match pid {
        0x42 => vec![0x41, 0x42, 0x30, 0x70], // 12.4 V, kept as bytes here.
        0x5c => vec![0x41, 0x5c, 0x82],       // 90 °C, kept as bytes here.
        0xa6 => vec![0x41, 0xa6, 0x00, 0x12, 0xd6, 0x87],
        // Battery system + pack readings require ISO-TP reassembly, not truncation.
        0xb6 => [vec![0x41, 0xb6, 0xff], (0..18).collect()].concat(),
        _ => vec![0x41, pid, 0x80, 0x00, 0xff], // Deliberately opaque raw block.
    }
}

fn can_reply(sender: u16, payload: &[u8]) -> String {
    let mut frames = Vec::new();
    if payload.len() <= 7 {
        let mut frame = vec![payload.len() as u8];
        frame.extend_from_slice(payload);
        frames.push(frame);
    } else {
        let mut first = vec![0x10 | (payload.len() >> 8) as u8, payload.len() as u8];
        first.extend_from_slice(&payload[..6]);
        frames.push(first);
        for (index, chunk) in payload[6..].chunks(7).enumerate() {
            let mut frame = vec![0x20 | ((index + 1) % 16) as u8];
            frame.extend_from_slice(chunk);
            frames.push(frame);
        }
    }
    frames
        .into_iter()
        .map(|mut frame| {
            frame.resize(8, 0);
            format!(
                "{sender:03X} {}\r",
                frame
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        })
        .collect()
}

#[test]
fn every_raw_catalog_selection_sends_its_own_pid_and_preserves_complete_ecu_payloads() {
    let directory = std::env::temp_dir().join(format!("kittx-pid-catalog-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("trip.kittx");
    let selected: BTreeSet<_> = capture_catalog()
        .into_iter()
        .filter(|item| !item.decoded)
        .map(|item| item.id)
        .collect();
    let expected = selected.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(b">").unwrap();
        let mut requests = BTreeSet::new();
        let mut command = Vec::new();
        let mut byte = [0];
        while stream.read(&mut byte).unwrap() != 0 {
            if byte[0] != b'\r' {
                command.push(byte[0]);
                continue;
            }
            let text = String::from_utf8_lossy(&command).trim().to_uppercase();
            let response = match text.as_str() {
                "ATI" => "ELM327 v2.1\r>".into(),
                "ATDPN" => "6\r>".into(),
                "ATRV" => "12.4V\r>".into(),
                "0902" => "7E8 10 14 49 02 01 57 44 42\r7E8 21 31 32 33 34 35 36 37\r7E8 22 38 39 30 31 32 33 34\r>".into(),
                s if s.starts_with("AT") => "OK\r>".into(),
                s if s.starts_with("01") && s.len() == 4 => {
                    let pid = u8::from_str_radix(&s[2..], 16).unwrap();
                    if pid % 32 == 0 {
                        format!("{}>", can_reply(0x7e8, &[0x41, pid, 0xff, 0xff, 0xff, 0xff]))
                    } else {
                        assert!(expected.contains(&u16::from(pid)), "unchecked request {s}");
                        requests.insert(u16::from(pid));
                        let mut response = can_reply(0x7e8, &payload(pid));
                        if pid == 0x42 {
                            response.push_str(&can_reply(0x7e9, &[0x41, 0x42, 0x32, 0x00]));
                        }
                        response.push('>');
                        response
                    }
                }
                _ => panic!("unexpected command {text}"),
            };
            stream.write_all(response.as_bytes()).unwrap();
            command.clear();
        }
        requests
    });
    let settings = CaptureSettings {
        transport: TransportConfig::tcp("127.0.0.1", port),
        selected: selected.clone(),
        dtcs: [false; 3],
        sample_period: Duration::from_secs(3600),
        operation_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let mut job = RecordingJob::spawn(settings, path.clone()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !job.is_finished() {
        assert!(Instant::now() < deadline, "{:?}", job.snapshot());
        if job.snapshot().phase == Phase::Recording {
            let reader = RecordingReader::open(&path).unwrap();
            // One response per raw PID, plus the independent peer ECU for PID 42.
            if reader
                .measurements(&Query {
                    limit: 1000,
                    ..Default::default()
                })
                .unwrap()
                .len()
                == selected.len() + 1
            {
                job.request_stop();
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(job.reap());
    assert_eq!(
        job.snapshot().phase,
        Phase::Completed,
        "{:?}",
        job.snapshot()
    );
    assert_eq!(job.snapshot().progress.failed_acquisitions, 0);
    assert_eq!(server.join().unwrap(), selected);
    let reader = RecordingReader::open(&path).unwrap();
    let selections = reader.selections().unwrap();
    assert_eq!(selections.len(), selected.len());
    assert!(selections.iter().all(|s| s.capability == "supported"));
    let rows = reader
        .measurements(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(rows.len(), selected.len() + 1);
    for pid in selected {
        let samples: Vec<_> = rows
            .iter()
            .filter(|row| row.signal_id == format!("01:{pid:02X}:raw"))
            .collect();
        assert!(!samples.is_empty(), "missing recorded PID {pid:02X}");
        assert!(
            samples
                .iter()
                .any(|row| row.bytes.as_deref() == Some(payload(pid as u8).as_slice()))
        );
        assert!(
            samples
                .iter()
                .all(|row| row.number.is_none() && row.unit == "raw")
        );
    }
    let peers: Vec<_> = rows
        .iter()
        .filter(|row| row.signal_id == "01:42:raw")
        .collect();
    assert_eq!(peers.len(), 2);
    assert_ne!(peers[0].ecu_id, peers[1].ecu_id);
    assert!(
        peers
            .iter()
            .any(|row| row.bytes.as_deref() == Some(&[0x41, 0x42, 0x32, 0x00]))
    );
    drop(reader);
    drop(job);
    std::fs::remove_dir_all(directory).unwrap();
}
