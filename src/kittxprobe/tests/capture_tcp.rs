mod common;
use common::ScriptServer;
use kittx::{
    TransportConfig,
    datastore::{Query, RecordingReader},
};
use kittx_probe::capture::ProbeCapture;
use kittx_recorder::{model::CaptureSettings, recording::Phase};
use std::time::{Duration, Instant};

#[test]
fn start_live_selected_pids_and_dtcs_stop_save_and_open_in_viewer() {
    let server = ScriptServer::start();
    let mut capture = ProbeCapture::start(CaptureSettings {
        transport: TransportConfig::tcp("127.0.0.1", server.port),
        selected: [0x0c, 0x61].into(),
        dtcs: [true, false, true],
        sample_period: Duration::from_millis(10),
        dtc_period: Duration::from_millis(40),
        operation_timeout: Duration::from_secs(1),
        ..Default::default()
    })
    .unwrap();
    let path = capture.snapshot().path;
    let root = path.parent().unwrap().to_owned();
    let output = root.join("saved.SQLITE");
    assert!(capture.saved_to().is_none());
    assert!(!capture.can_save());
    assert!(!capture.reap());
    assert!(capture.save(&output).unwrap_err().contains("Stop"));
    assert!(!output.with_extension("kittx").exists());
    let live_reader = capture.reader();
    let deadline = Instant::now() + Duration::from_secs(8);
    let live = loop {
        let status = capture.snapshot();
        assert_ne!(status.phase, Phase::Failed, "{}", status.message);
        if status.progress.committed > 0
            && let Ok(live) = live_reader.read(true, 400)
            && live.tracks.len() == 2
            && live.dtcs.iter().any(|d| d.scan.category == "permanent")
        {
            break live;
        }
        assert!(
            Instant::now() < deadline,
            "capture did not produce live data"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(!capture.is_finished());
    assert!(
        live.tracks
            .iter()
            .find(|t| t.unit == "rpm")
            .unwrap()
            .signals[0]
            .points
            .iter()
            .all(|p| p.value == 1725.)
    );
    assert!(
        live.tracks.iter().find(|t| t.unit == "%").unwrap().signals[0]
            .points
            .iter()
            .all(|p| p.value == 3.)
    );
    assert!(live.dtcs.iter().any(|d| d.codes == ["P1337"]));
    let requested = live.view.zoom(0.5, 0.5);
    let detail = live_reader.read_view(true, 400, Some(requested)).unwrap();
    assert_eq!(detail.view, requested);
    assert_eq!(detail.tracks.len(), 2);
    capture.request_stop();
    while !capture.is_finished() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(capture.reap());
    assert!(capture.can_save());
    assert_eq!(capture.snapshot().phase, Phase::Completed);
    assert!(
        live_reader.read(true, 400).is_err(),
        "finalization permanently closes old live reader leases"
    );
    assert!(live_reader.read_view(true, 400, Some(requested)).is_err());
    assert!(
        capture
            .reader()
            .read_view(false, 400, Some(requested))
            .is_ok()
    );
    assert!(
        capture.reader().read(false, 400).is_ok(),
        "a stopped snapshot may read the finalized file"
    );
    let requests = server.finish();
    for request in ["010C", "0161", "03", "0A"] {
        assert!(requests.iter().any(|r| r == request), "{request}");
    }
    for request in ["010D", "07", "04"] {
        assert!(!requests.iter().any(|r| r == request), "{request}");
    }
    let saved = capture.save(&output).unwrap();
    assert_eq!(capture.saved_to(), Some(saved.as_path()));
    assert!(capture.save(&saved).is_err());
    let doc = kittx_viewer::document::Document::open(saved.clone()).unwrap();
    assert!(doc.overview.producer.starts_with("KittxProbe"));
    assert_eq!(doc.overview.session.status, "completed");
    assert_eq!(doc.overview.series.iter().filter(|s| s.numeric).count(), 2);
    let reader = RecordingReader::open(&saved).unwrap();
    let scans = reader
        .dtc_scans(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert!(
        scans
            .iter()
            .all(|s| s.category == "stored" || s.category == "permanent")
    );
    assert!(path.is_file(), "working file remains recoverable");
    drop(reader);
    drop(capture);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_settings_never_start_a_worker() {
    assert!(
        ProbeCapture::start(CaptureSettings {
            selected: Default::default(),
            dtcs: [false; 3],
            ..Default::default()
        })
        .is_err()
    );
}
