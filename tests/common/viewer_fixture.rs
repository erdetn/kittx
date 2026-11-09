use kittx::datastore::*;
use rusqlite::{Connection, params};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub struct Fixture {
    pub path: PathBuf,
    pub db: Connection,
}
impl Fixture {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "viewer-{}-{}.kittx",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let db = Connection::open(&path).unwrap();
        db.execute_batch(include_str!("../../src/datastore/schema.sql"))
            .unwrap();
        db.pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        db.pragma_update(None, "user_version", SCHEMA_VERSION)
            .unwrap();
        db.execute_batch("INSERT INTO session(id,format_id,schema_version,created_utc_us,started_utc_us,status,producer,producer_version,library_version,platform,config,manufacturer_coverage) VALUES('test','kittx.ecu-recording',1,10000000,11000000,'recording','test','1','1','test','test','unknown');INSERT INTO vehicle VALUES(1,NULL,'failed');INSERT INTO connection_segment VALUES(1,10000000,0,NULL,'TCP','test');INSERT INTO ecu VALUES(1,1,6,'7E8');INSERT INTO ecu VALUES(2,1,6,'7E9');INSERT INTO signal_definition VALUES('speed',1,13,'km/h','1',0);INSERT INTO signal_definition VALUES('raw',1,99,'bytes','1',1);INSERT INTO capture_selection VALUES(1,'speed',NULL,1000,0,NULL,'supported');INSERT INTO dtc_scan_group VALUES(1,'manual',10000000,0,NULL,NULL,'all','partial');").unwrap();
        Self { path, db }
    }
    pub fn sample(&self, id: i64, value: f64) {
        self.db.execute("INSERT INTO acquisition VALUES(?1,1,'pid','010D',?2,?2,?2,?2,?2,'ok',NULL,NULL,NULL)",params![id,id*1000]).unwrap();
        self.db
            .execute(
                "INSERT INTO measurement VALUES(?1,?1,1,'speed',?2,?2,'number',?3,NULL,'km/h')",
                params![id, id * 1000, value],
            )
            .unwrap();
        self.db
            .execute("UPDATE session SET committed_sequence=?", [id])
            .unwrap();
    }
    pub fn scan(&self, id: i64, outcome: &str, count: Option<i64>) {
        self.db
            .execute(
                "INSERT INTO dtc_scan_result VALUES(?1,1,?1,1,'stored','OBD',NULL,?2,?3,NULL,NULL)",
                params![id, outcome, count],
            )
            .unwrap();
        if count.is_some_and(|c| c > 0) {
            self.db
                .execute(
                    "INSERT INTO dtc_entry VALUES(?1,?1,0,'P0123',X'0123',NULL,NULL)",
                    [id],
                )
                .unwrap();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
