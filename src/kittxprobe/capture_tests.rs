use super::{ReadGate, backup_database, save_recording};
use crate::test_fixture as fixture;
use kittx::datastore::{Query, RecordingReader};
use kittx_viewer::document::Document;

#[test]
fn save_includes_wal_normalizes_extension_and_preserves_metadata_and_source() {
    let fixture = fixture::Fixture::new();
    fixture
        .db
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    fixture.sample(1, 42.);
    fixture.scan(1, "ok", Some(1));
    fixture
        .db
        .execute_batch("UPDATE session SET status='completed',producer='KittxProbe'")
        .unwrap();
    let before = std::fs::read(&fixture.path).unwrap();
    let destination = fixture.path.with_extension("export.SQLITE");
    let saved = save_recording(&fixture.path, &destination).unwrap();
    assert_eq!(saved.extension().unwrap(), "kittx");
    let reader = RecordingReader::open(&saved).unwrap();
    assert_eq!(reader.session().unwrap().status, "completed");
    assert_eq!(
        reader.measurements(&Query::default()).unwrap()[0].number,
        Some(42.)
    );
    assert_eq!(
        reader.dtc_entries(1, &Query::default()).unwrap()[0].code,
        "P0123"
    );
    let doc = Document::open(saved.clone()).unwrap();
    assert!(doc.overview.producer.starts_with("KittxProbe"));
    assert_eq!(doc.overview.session.committed_sequence, 1);
    assert_eq!(
        doc.frame(doc.full(), &[0], 100, Default::default())
            .unwrap()
            .samples[0]
            .1[0]
            .value,
        42.
    );
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    let c = rusqlite::Connection::open(&saved).unwrap();
    assert_eq!(
        c.pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
            .unwrap(),
        "delete"
    );
    drop(c);
    drop(reader);
    std::fs::remove_file(saved).unwrap();
}

#[test]
fn save_rejects_active_invalid_and_existing_files_and_allows_retry() {
    let fixture = fixture::Fixture::new();
    fixture.sample(1, 42.);
    let target = fixture.path.with_extension("saved.kittx");
    assert!(
        save_recording(&fixture.path, &target)
            .unwrap_err()
            .contains("not finalized")
    );
    assert!(!target.exists());
    fixture
        .db
        .execute_batch("UPDATE session SET status='start_failed'")
        .unwrap();
    std::fs::write(&target, b"keep existing file").unwrap();
    assert!(save_recording(&fixture.path, &target).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"keep existing file");
    assert!(save_recording(&fixture.path, &fixture.path.join("missing/target.kittx")).is_err());
    assert!(save_recording(&target, &target.with_extension("retry.kittx")).is_err());
    let retry = save_recording(&fixture.path, &target.with_extension("retry.kittx")).unwrap();
    assert_eq!(
        RecordingReader::open(&retry)
            .unwrap()
            .session()
            .unwrap()
            .status,
        "start_failed"
    );
    std::fs::remove_file(retry).unwrap();
    std::fs::remove_file(target).unwrap();
}

#[test]
fn sqlite_full_during_backup_preserves_the_working_recording_and_allows_retry() {
    let fixture = fixture::Fixture::new();
    fixture.sample(1, 42.);
    fixture
        .db
        .execute("UPDATE session SET status='interrupted'", [])
        .unwrap();
    let before = std::fs::read(&fixture.path).unwrap();
    let source = rusqlite::Connection::open_with_flags(
        &fixture.path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut limited = rusqlite::Connection::open_in_memory().unwrap();
    limited.pragma_update(None, "max_page_count", 1).unwrap();
    assert!(
        backup_database(&source, &mut limited)
            .unwrap_err()
            .contains("full")
    );
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    let saved = save_recording(&fixture.path, &fixture.path.with_extension("retry.kittx")).unwrap();
    let reader = RecordingReader::open(&saved).unwrap();
    assert_eq!(reader.session().unwrap().status, "interrupted");
    assert_eq!(
        reader.measurements(&Query::default()).unwrap()[0].number,
        Some(42.)
    );
    drop(reader);
    std::fs::remove_file(saved).unwrap();
}

#[test]
fn finalization_barrier_waits_for_existing_readers_and_rejects_new_live_reads() {
    use std::{
        sync::{Arc, mpsc},
        time::Duration,
    };
    let gate = Arc::new(ReadGate::default());
    let lease = gate.enter().unwrap();
    let barrier = gate.clone();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        barrier.pause();
        done_tx.send(()).unwrap();
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !gate.state.lock().unwrap().paused {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(gate.enter().is_err());
    assert!(
        done_rx.try_recv().is_err(),
        "finalization must wait while a lease is alive"
    );
    drop(lease);
    done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert!(gate.enter().is_err());
}
