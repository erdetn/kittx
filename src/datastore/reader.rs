use super::store::validate;
use super::*;
use rusqlite::{Connection, OpenFlags, params};
use std::{path::Path, time::Duration};

/// Keyset pagination: set after_id to the last returned row ID. Maximum 1000 rows.
/// Time filters are inclusive elapsed microseconds; ECU identity is its database ID.
#[derive(Debug, Clone)]
pub struct Query {
    pub after_id: i64,
    pub limit: u32,
    pub from_elapsed_us: Option<i64>,
    pub to_elapsed_us: Option<i64>,
    pub ecu_id: Option<i64>,
    pub signal_id: Option<String>,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            after_id: 0,
            limit: 100,
            from_elapsed_us: None,
            to_elapsed_us: None,
            ecu_id: None,
            signal_id: None,
        }
    }
}
impl Query {
    fn validate(&self) -> DataResult<()> {
        if self.after_id < 0
            || self.limit == 0
            || self.limit > 1000
            || self.from_elapsed_us.is_some_and(|t| t < 0)
            || self.to_elapsed_us.is_some_and(|t| t < 0)
            || self
                .from_elapsed_us
                .zip(self.to_elapsed_us)
                .is_some_and(|(a, b)| a > b)
        {
            return Err(invalid("invalid query page/time bounds"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub status: String,
    pub created_utc_us: i64,
    pub started_utc_us: Option<i64>,
    pub ended_utc_us: Option<i64>,
    pub finalized_utc_us: Option<i64>,
    pub committed_sequence: i64,
    pub vin: Option<String>,
    pub vin_status: String,
    pub manufacturer_coverage: String,
    pub ending_scan_complete: bool,
    pub exported_utc_us: Option<i64>,
}
#[derive(Debug, Clone)]
pub struct VehicleField {
    pub name: String,
    pub value: Option<String>,
    pub source: Option<String>,
    pub source_version: Option<String>,
    pub observed_utc_us: i64,
}
#[derive(Debug, Clone)]
pub struct MeasurementRow {
    pub id: i64,
    pub acquisition_id: i64,
    pub ecu_id: i64,
    pub signal_id: String,
    pub utc_us: i64,
    pub elapsed_us: i64,
    pub number: Option<f64>,
    pub bytes: Option<Vec<u8>>,
    pub unit: String,
}
#[derive(Debug, Clone)]
pub struct AcquisitionRow {
    pub id: i64,
    pub kind: String,
    pub request: String,
    pub due_elapsed_us: i64,
    pub started: Timestamp,
    pub completed: Timestamp,
    pub outcome: String,
    pub error: Option<String>,
    pub recovery: Option<String>,
    pub target: Option<String>,
}
#[derive(Debug, Clone)]
pub struct DtcScanRow {
    pub id: i64,
    pub group_id: i64,
    pub acquisition_id: i64,
    pub ecu_id: Option<i64>,
    pub category: String,
    pub namespace: String,
    pub profile_id: Option<String>,
    pub outcome: String,
    pub code_count: Option<i64>,
    pub raw_payload: Option<Vec<u8>>,
    pub error: Option<String>,
    pub utc_us: i64,
    pub elapsed_us: i64,
}
#[derive(Debug, Clone)]
pub struct DtcEntryRow {
    pub id: i64,
    pub scan_id: i64,
    pub ordinal: i64,
    pub code: String,
    pub bytes: Vec<u8>,
    pub status: Option<Vec<u8>>,
    pub interpretation: Option<String>,
}
#[derive(Debug, Clone)]
pub struct EventRow {
    pub id: i64,
    pub utc_us: i64,
    pub elapsed_us: i64,
    pub kind: String,
    pub detail: String,
}
#[derive(Debug, Clone)]
pub struct EvidenceRow {
    pub acquisition_id: i64,
    pub scope: String,
    pub raw_response: Option<Vec<u8>>,
    pub attempted_request: Option<Vec<u8>>,
    pub protocol: Option<u8>,
    pub context: String,
    pub partial: bool,
    pub truncated: bool,
}
#[derive(Debug, Clone)]
pub struct EcuRow {
    pub id: i64,
    pub segment_id: i64,
    pub protocol: i32,
    pub sender: String,
}
#[derive(Debug, Clone)]
pub struct SelectionRow {
    pub id: i64,
    pub signal_id: String,
    pub target: Option<String>,
    pub period_us: i64,
    pub capability: String,
}
#[derive(Debug, Clone)]
pub struct ScanGroupRow {
    pub id: i64,
    pub trigger: String,
    pub started: Timestamp,
    pub ended_utc_us: Option<i64>,
    pub requested: String,
    pub coverage: String,
}

/// Bounded read-only queries. Each query releases its SQLite read transaction before returning.
pub struct RecordingReader {
    pub(crate) connection: Connection,
}
impl RecordingReader {
    pub fn open(path: impl AsRef<Path>) -> DataResult<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_millis(250))?;
        connection.pragma_update(None, "trusted_schema", false)?;
        validate(&connection)?;
        Ok(Self { connection })
    }
    /// Explicit recovery only after the original recorder/process has stopped.
    /// Keep its WAL sidecars together with the original file. Does not resume acquisition.
    pub fn recover_interrupted(path: impl AsRef<Path>) -> DataResult<Self> {
        let connection =
            Connection::open_with_flags(path.as_ref(), OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.busy_timeout(Duration::from_millis(250))?;
        connection.pragma_update(None, "trusted_schema", false)?;
        validate(&connection)?;
        let check: String = connection.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(invalid(format!("integrity check failed: {check}")));
        }
        if connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
            return Err(invalid("foreign key check failed"));
        }
        connection.execute("UPDATE session SET status='interrupted' WHERE status IN ('preparing','recording','finalizing')",[])?;
        connection.close().map_err(|(_, e)| DataError::Sqlite(e))?;
        Self::open(path)
    }
    pub fn session(&self) -> DataResult<SessionInfo> {
        Ok(self.connection.query_row("SELECT s.id,s.status,s.created_utc_us,s.started_utc_us,s.ended_utc_us,s.finalized_utc_us,s.committed_sequence,v.vin,v.vin_status,s.manufacturer_coverage,s.ending_scan_complete,s.exported_utc_us FROM session s CROSS JOIN vehicle v",[],|r|Ok(SessionInfo{id:r.get(0)?,status:r.get(1)?,created_utc_us:r.get(2)?,started_utc_us:r.get(3)?,ended_utc_us:r.get(4)?,finalized_utc_us:r.get(5)?,committed_sequence:r.get(6)?,vin:r.get(7)?,vin_status:r.get(8)?,manufacturer_coverage:r.get(9)?,ending_scan_complete:r.get(10)?,exported_utc_us:r.get(11)?}))?)
    }
    pub fn vehicle_fields(&self) -> DataResult<Vec<VehicleField>> {
        let mut statement=self.connection.prepare("SELECT name,value,source,source_version,observed_utc_us FROM vehicle_field ORDER BY name")?;
        Ok(statement
            .query_map([], |r| {
                Ok(VehicleField {
                    name: r.get(0)?,
                    value: r.get(1)?,
                    source: r.get(2)?,
                    source_version: r.get(3)?,
                    observed_utc_us: r.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?)
    }
    pub fn measurements(&self, q: &Query) -> DataResult<Vec<MeasurementRow>> {
        q.validate()?;
        let mut s=self.connection.prepare("SELECT id,acquisition_id,ecu_id,signal_id,observed_utc_us,observed_elapsed_us,number_value,blob_value,unit FROM measurement WHERE id>? AND (? IS NULL OR observed_elapsed_us>=?) AND (? IS NULL OR observed_elapsed_us<=?) AND (? IS NULL OR ecu_id=?) AND (? IS NULL OR signal_id=?) ORDER BY id LIMIT ?")?;
        Ok(s.query_map(
            params![
                q.after_id,
                q.from_elapsed_us,
                q.from_elapsed_us,
                q.to_elapsed_us,
                q.to_elapsed_us,
                q.ecu_id,
                q.ecu_id,
                q.signal_id,
                q.signal_id,
                q.limit
            ],
            |r| {
                Ok(MeasurementRow {
                    id: r.get(0)?,
                    acquisition_id: r.get(1)?,
                    ecu_id: r.get(2)?,
                    signal_id: r.get(3)?,
                    utc_us: r.get(4)?,
                    elapsed_us: r.get(5)?,
                    number: r.get(6)?,
                    bytes: r.get(7)?,
                    unit: r.get(8)?,
                })
            },
        )?
        .collect::<Result<_, _>>()?)
    }
    pub fn acquisitions(&self, q: &Query) -> DataResult<Vec<AcquisitionRow>> {
        q.validate()?;
        let mut s=self.connection.prepare("SELECT id,kind,request,due_elapsed_us,operation_started_utc_us,operation_started_elapsed_us,operation_completed_utc_us,operation_completed_elapsed_us,outcome,error,recovery,target FROM acquisition WHERE id>? AND (? IS NULL OR operation_completed_elapsed_us>=?) AND (? IS NULL OR operation_completed_elapsed_us<=?) ORDER BY id LIMIT ?")?;
        Ok(s.query_map(
            params![
                q.after_id,
                q.from_elapsed_us,
                q.from_elapsed_us,
                q.to_elapsed_us,
                q.to_elapsed_us,
                q.limit
            ],
            |r| {
                Ok(AcquisitionRow {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    request: r.get(2)?,
                    due_elapsed_us: r.get(3)?,
                    started: Timestamp {
                        utc_us: r.get(4)?,
                        elapsed_us: r.get(5)?,
                    },
                    completed: Timestamp {
                        utc_us: r.get(6)?,
                        elapsed_us: r.get(7)?,
                    },
                    outcome: r.get(8)?,
                    error: r.get(9)?,
                    recovery: r.get(10)?,
                    target: r.get(11)?,
                })
            },
        )?
        .collect::<Result<_, _>>()?)
    }
    pub fn dtc_scans(&self, q: &Query) -> DataResult<Vec<DtcScanRow>> {
        q.validate()?;
        let mut s=self.connection.prepare("SELECT d.id,d.group_id,d.acquisition_id,d.ecu_id,d.category,d.namespace,d.profile_id,d.outcome,d.code_count,d.raw_payload,d.error,a.operation_completed_utc_us,a.operation_completed_elapsed_us FROM dtc_scan_result d JOIN acquisition a ON a.id=d.acquisition_id WHERE d.id>? AND (? IS NULL OR a.operation_completed_elapsed_us>=?) AND (? IS NULL OR a.operation_completed_elapsed_us<=?) AND (? IS NULL OR d.ecu_id=?) ORDER BY d.id LIMIT ?")?;
        Ok(s.query_map(
            params![
                q.after_id,
                q.from_elapsed_us,
                q.from_elapsed_us,
                q.to_elapsed_us,
                q.to_elapsed_us,
                q.ecu_id,
                q.ecu_id,
                q.limit
            ],
            |r| {
                Ok(DtcScanRow {
                    id: r.get(0)?,
                    group_id: r.get(1)?,
                    acquisition_id: r.get(2)?,
                    ecu_id: r.get(3)?,
                    category: r.get(4)?,
                    namespace: r.get(5)?,
                    profile_id: r.get(6)?,
                    outcome: r.get(7)?,
                    code_count: r.get(8)?,
                    raw_payload: r.get(9)?,
                    error: r.get(10)?,
                    utc_us: r.get(11)?,
                    elapsed_us: r.get(12)?,
                })
            },
        )?
        .collect::<Result<_, _>>()?)
    }
    pub fn dtc_entries(&self, scan_id: i64, q: &Query) -> DataResult<Vec<DtcEntryRow>> {
        q.validate()?;
        let mut s=self.connection.prepare("SELECT id,scan_id,ordinal,code,code_bytes,status_bytes,interpretation FROM dtc_entry WHERE scan_id=? AND id>? ORDER BY id LIMIT ?")?;
        Ok(s.query_map(params![scan_id, q.after_id, q.limit], |r| {
            Ok(DtcEntryRow {
                id: r.get(0)?,
                scan_id: r.get(1)?,
                ordinal: r.get(2)?,
                code: r.get(3)?,
                bytes: r.get(4)?,
                status: r.get(5)?,
                interpretation: r.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?)
    }
    pub fn evidence(&self, acquisition_id: i64) -> DataResult<EvidenceRow> {
        Ok(self.connection.query_row("SELECT acquisition_id,scope,raw_response,protocol,context,partial,truncated,attempted_request FROM transaction_evidence WHERE acquisition_id=?",[acquisition_id],|r|Ok(EvidenceRow{acquisition_id:r.get(0)?,scope:r.get(1)?,raw_response:r.get(2)?,protocol:r.get(3)?,context:r.get(4)?,partial:r.get(5)?,truncated:r.get(6)?,attempted_request:r.get(7)?}))?)
    }
    pub fn events(&self, q: &Query) -> DataResult<Vec<EventRow>> {
        q.validate()?;
        let mut s=self.connection.prepare("SELECT id,utc_us,elapsed_us,kind,detail FROM event WHERE id>? AND (? IS NULL OR elapsed_us>=?) AND (? IS NULL OR elapsed_us<=?) ORDER BY id LIMIT ?")?;
        Ok(s.query_map(
            params![
                q.after_id,
                q.from_elapsed_us,
                q.from_elapsed_us,
                q.to_elapsed_us,
                q.to_elapsed_us,
                q.limit
            ],
            |r| {
                Ok(EventRow {
                    id: r.get(0)?,
                    utc_us: r.get(1)?,
                    elapsed_us: r.get(2)?,
                    kind: r.get(3)?,
                    detail: r.get(4)?,
                })
            },
        )?
        .collect::<Result<_, _>>()?)
    }
    pub fn ecus(&self, q: &Query) -> DataResult<Vec<EcuRow>> {
        q.validate()?;
        let mut s = self.connection.prepare(
            "SELECT id,segment_id,protocol,sender FROM ecu WHERE id>? ORDER BY id LIMIT ?",
        )?;
        Ok(s.query_map(params![q.after_id, q.limit], |r| {
            Ok(EcuRow {
                id: r.get(0)?,
                segment_id: r.get(1)?,
                protocol: r.get(2)?,
                sender: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?)
    }
    pub fn selections(&self) -> DataResult<Vec<SelectionRow>> {
        let mut s=self.connection.prepare("SELECT id,signal_id,target,period_us,capability FROM capture_selection ORDER BY id LIMIT 256")?;
        Ok(s.query_map([], |r| {
            Ok(SelectionRow {
                id: r.get(0)?,
                signal_id: r.get(1)?,
                target: r.get(2)?,
                period_us: r.get(3)?,
                capability: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?)
    }
    pub fn scan_groups(&self, q: &Query) -> DataResult<Vec<ScanGroupRow>> {
        q.validate()?;
        let mut s=self.connection.prepare("SELECT id,trigger,started_utc_us,started_elapsed_us,ended_utc_us,requested,coverage FROM dtc_scan_group WHERE id>? AND (? IS NULL OR started_elapsed_us>=?) AND (? IS NULL OR started_elapsed_us<=?) ORDER BY id LIMIT ?")?;
        Ok(s.query_map(
            params![
                q.after_id,
                q.from_elapsed_us,
                q.from_elapsed_us,
                q.to_elapsed_us,
                q.to_elapsed_us,
                q.limit
            ],
            |r| {
                Ok(ScanGroupRow {
                    id: r.get(0)?,
                    trigger: r.get(1)?,
                    started: Timestamp {
                        utc_us: r.get(2)?,
                        elapsed_us: r.get(3)?,
                    },
                    ended_utc_us: r.get(4)?,
                    requested: r.get(5)?,
                    coverage: r.get(6)?,
                })
            },
        )?
        .collect::<Result<_, _>>()?)
    }
}
