//! Local deterministic ELM327 transcript server; no external network or hardware.
use kittx::TransportConfig;
use kittx_recorder::{
    model::CaptureSettings,
    recording::{Phase, RecordingJob},
};
use kittx_viewer::document::Document;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
#[test]
fn real_worker_tcp_timed_and_manual_recordings_open_in_viewer() {
    for timed in [true, false] {
        let directory = std::env::temp_dir().join(format!(
            "kittx-worker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("trip.kittx");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            if timed {
                stream.write_all(b">").unwrap();
            }
            let mut startup_pending = !timed;
            let mut command = Vec::new();
            let mut requests = Vec::new();
            let mut byte = [0];
            loop {
                match stream.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => {
                        if byte[0] != b'\r' {
                            command.push(byte[0]);
                            continue;
                        }
                        let text = String::from_utf8_lossy(&command).trim().to_uppercase();
                        requests.push(text.clone());
                        if startup_pending {
                            // Deterministically withhold startup until the read-only
                            // synchronization probe. Its reply must also be consumed.
                            assert_eq!(text, "ATRV");
                            stream.write_all(b"ELM327 v2.1\r>").unwrap();
                            startup_pending = false;
                        }
                        let response = match text.as_str() {
                            "ATI" => "ELM327 v2.1\r>",
                            "ATDPN" => "6\r>",
                            "ATRV" => "12.4V\r>",
                            "0902" => {
                                "7E8 10 14 49 02 01 57 44 42\r7E8 21 31 32 33 34 35 36 37\r7E8 22 38 39 30 31 32 33 34\r>"
                            }
                            "0100" => "7E8 06 41 00 FF FF FF FF 00\r>",
                            "010C" => "7E8 04 41 0C 1A F4 00 00 00\r>",
                            "03" => "7E8 04 43 01 13 37 00 00 00\r>",
                            "07" => "7E8 02 47 00 00 00 00 00 00\r>",
                            "0A" => "7E8 04 4A 01 13 37 00 00 00\r>",
                            s if s.starts_with("AT") => "OK\r>",
                            _ => "NO DATA\r>",
                        };
                        stream.write_all(response.as_bytes()).unwrap();
                        command.clear();
                    }
                    Err(error) => panic!("scripted TCP read failed: {error}"),
                }
            }
            requests
        });
        let settings = CaptureSettings {
            transport: TransportConfig::tcp("127.0.0.1", port),
            selected: [0x0c].into_iter().collect(),
            duration: timed.then_some(Duration::from_millis(80)),
            sample_period: Duration::from_millis(10),
            operation_timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let mut job = RecordingJob::spawn(settings, path.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut stopped = false;
        while !job.is_finished() {
            assert!(
                Instant::now() < deadline,
                "worker did not finish: {:?}",
                job.snapshot()
            );
            let status = job.snapshot();
            if !timed
                && !stopped
                && status.phase == Phase::Recording
                && status.progress.acquired >= 8
            {
                job.request_stop();
                stopped = true;
                assert_eq!(job.snapshot().phase, Phase::Stopping);
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(job.reap());
        let status = job.snapshot();
        assert_eq!(status.phase, Phase::Completed, "{status:?}");
        let log = std::fs::read_to_string(status.diagnostics_path.as_ref().unwrap()).unwrap();
        assert!(log.contains("Opening Tcp") && log.contains("Adapter ready"));
        assert!(log.contains("Read Vin") && log.contains("Recording finalized"));
        assert!(status.last_error.is_none(), "{status:?}");
        assert!(status.summary.unwrap().ending_scan_complete);
        assert_eq!(status.vin.as_deref(), Some("WDB12345678901234"));
        let requests = server.join().unwrap();
        assert!(requests.iter().any(|request| request == "010C"));
        assert!(
            requests
                .iter()
                .filter(|request| request.starts_with("01"))
                .all(|request| matches!(request.as_str(), "0100" | "010C")),
            "unchecked PID requested: {requests:?}"
        );
        let doc = Document::open(path).unwrap();
        assert_eq!(doc.overview.session.status, "completed");
        assert_eq!(doc.overview.series.len(), 1);
        assert_eq!(doc.overview.series[0].signal, "01:0C");
        let frame = doc
            .frame(doc.full(), &[0], 100, Default::default())
            .unwrap();
        assert!(frame.samples[0].1.iter().any(|p| p.value == 1725.));
        assert!(!frame.scans.is_empty());
        drop(doc);
        drop(job);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

/// Explicit smoke test for a separately running emulator; ordinary tests remain isolated.
#[test]
#[ignore = "requires the running ELM327 emulator at 127.0.0.1:36000"]
fn running_emulator_records_selected_signals_and_dtcs_with_diagnostics() {
    let directory =
        std::env::temp_dir().join(format!("kittx-live-emulator-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("trip.kittx");
    let settings = CaptureSettings {
        duration: Some(Duration::from_secs(2)),
        ..Default::default()
    };
    let mut job = RecordingJob::spawn(settings, path.clone()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !job.is_finished() {
        assert!(Instant::now() < deadline, "{:?}", job.snapshot());
        thread::sleep(Duration::from_millis(5));
    }
    assert!(job.reap());
    let status = job.snapshot();
    assert_eq!(status.phase, Phase::Completed, "{status:?}");
    assert_eq!(status.progress.failed_acquisitions, 0, "{status:?}");
    assert!(status.summary.as_ref().unwrap().ending_scan_complete);
    assert_eq!(status.vin.as_deref(), Some("WDB12345678901234"));
    assert!(status.last_error.is_none(), "{status:?}");
    let reader = kittx::datastore::RecordingReader::open(&path).unwrap();
    let scans = reader.dtc_scans(&Default::default()).unwrap();
    for category in ["stored", "pending", "permanent"] {
        assert!(
            scans
                .iter()
                .any(|scan| scan.category == category && scan.error.is_none())
        );
    }
    let document = Document::open(path).unwrap();
    assert_eq!(document.overview.session.status, "completed");
    assert_eq!(document.overview.series.len(), 8);
    let indices: Vec<_> = (0..8).collect();
    let frame = document
        .frame(document.full(), &indices, 100, Default::default())
        .unwrap();
    assert!(frame.samples.iter().all(|(_, values)| !values.is_empty()));
    assert!(!frame.scans.is_empty());
    println!(
        "Emulator smoke test passed; recordings and diagnostics: {}",
        directory.display()
    );
    // Keep this explicitly requested smoke test's artifacts for inspection.
}
