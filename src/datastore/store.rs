use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) struct Pending {
    pub sequence: i64,
    pub request: ReadRequest,
    pub due: i64,
    pub start: Timestamp,
    pub end: Timestamp,
    pub observation: Observation,
    pub scan_group: Option<i64>,
    pub target: Option<crate::obd::Sender>,
    pub truncated: bool,
}
pub(crate) struct Store {
    pub connection: Option<Connection>,
    pub path: PathBuf,
    pub id: String,
    pub segment: Option<i64>,
}
pub(crate) fn new_file(path: &Path) -> DataResult<Connection> {
    if path.extension().and_then(|s| s.to_str()) != Some("kittx") {
        return Err(invalid("recording filename must end in .kittx"));
    }
    OpenOptions::new().write(true).create_new(true).open(path)?;
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::from_millis(250))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    Ok(connection)
}
pub(crate) fn validate(connection: &Connection) -> DataResult<()> {
    let id: i32 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if id != APPLICATION_ID || version != SCHEMA_VERSION {
        return Err(invalid("not a supported .kittx schema/application ID"));
    }
    let (format, version, count): (String, i32, i64) = connection.query_row(
        "SELECT format_id,schema_version,(SELECT count(*) FROM session) FROM session",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if format != FORMAT_ID || version != SCHEMA_VERSION || count != 1 {
        return Err(invalid("invalid .kittx session identity"));
    }
    Ok(())
}
impl Store {
    pub fn db(&self) -> DataResult<&Connection> {
        self.connection
            .as_ref()
            .ok_or_else(|| DataError::State("recording is closed".into()))
    }
    pub fn create(path: &Path, config: &RecordingConfig, time: Timestamp) -> DataResult<Self> {
        let connection = new_file(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "wal_autocheckpoint", 256)?;
        let mode: String = connection.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        let sync: i32 = connection.pragma_query_value(None, "synchronous", |r| r.get(0))?;
        let fk: bool = connection.pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
        if mode != "wal" || sync != 2 || !fk {
            return Err(invalid("SQLite durability configuration was not applied"));
        }
        let id: String =
            connection.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
        let tx = connection.unchecked_transaction()?;
        tx.execute_batch(include_str!("schema.sql"))?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.execute("INSERT INTO session(id,format_id,schema_version,created_utc_us,previous_session,status,producer,producer_version,library_version,platform,config,manufacturer_coverage) VALUES(?,?,?,?,?,'preparing',?,?,?,?,?,?)",params![id,FORMAT_ID,SCHEMA_VERSION,time.utc_us,config.previous_session,config.producer,config.producer_version,env!("CARGO_PKG_VERSION"),format!("{}-{}",std::env::consts::OS,std::env::consts::ARCH),format!("{config:?}"),if config.profiles.is_empty(){"not_configured"}else{"attempted"}])?;
        let (commit_mode, age, count, bytes) = match config.commit_policy {
            CommitPolicy::EachAcquisition => ("each_acquisition", None, None, None),
            CommitPolicy::Batch {
                max_age,
                max_acquisitions,
                max_bytes,
            } => (
                "batch",
                Some(micros(max_age)?),
                Some(max_acquisitions as i64),
                Some(max_bytes as i64),
            ),
        };
        tx.execute(
            "INSERT INTO recording_policy VALUES(1,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                config.duration.map(micros).transpose()?,
                micros(config.operation_timeout)?,
                micros(config.shutdown_timeout)?,
                commit_mode,
                age,
                count,
                bytes,
                config.max_observation_bytes as i64,
                config.dtcs.as_ref().map(|d| micros(d.period)).transpose()?,
                config.dtcs.as_ref().is_some_and(|d| d.beginning),
                config.dtcs.as_ref().is_some_and(|d| d.ending)
            ],
        )?;
        if let Some(dtcs) = &config.dtcs {
            for kind in &dtcs.kinds {
                tx.execute("INSERT INTO dtc_selection VALUES(?)", [kind.service()])?;
            }
        }
        tx.execute("INSERT INTO vehicle VALUES(1,NULL,'pending')", [])?;
        for (name, value) in config.vehicle.fields() {
            tx.execute(
                "INSERT INTO vehicle_field VALUES(?,?,?,?,?)",
                params![
                    name,
                    value.as_ref().map(|v| &v.value),
                    value.as_ref().map(|v| &v.source),
                    value.as_ref().and_then(|v| v.source_version.as_ref()),
                    time.utc_us
                ],
            )?;
        }
        for (index, selection) in config.signals.iter().enumerate() {
            let signal = selection.signal;
            tx.execute(
                "INSERT OR IGNORE INTO signal_definition VALUES(?,?,?,?,?,?)",
                params![
                    signal.id(),
                    signal.pid().map(|_| 1),
                    signal.pid(),
                    signal.unit(),
                    env!("CARGO_PKG_VERSION"),
                    matches!(signal, Signal::RawPid(_))
                ],
            )?;
            tx.execute(
                "INSERT INTO capture_selection VALUES(?,?,?,?,?,NULL,'unknown')",
                params![
                    index as i64 + 1,
                    signal.id(),
                    selection.target.map(sender_key),
                    micros(selection.period)?,
                    time.elapsed_us
                ],
            )?;
        }
        for (index, selection) in config.signals.iter().enumerate() {
            tx.execute("INSERT INTO selection_change(selection_id,utc_us,elapsed_us,period_us) VALUES(?,?,?,?)",params![index as i64+1,time.utc_us,time.elapsed_us,micros(selection.period)?])?;
        }
        for p in &config.profiles {
            tx.execute(
                "INSERT INTO diagnostic_profile VALUES(?,?,?,?,?,?,?)",
                params![
                    p.id,
                    p.version,
                    p.namespace,
                    p.request,
                    p.target.map(sender_key),
                    p.header.map(|h| format!("{h:?}")),
                    p.receive_filter
                ],
            )?;
        }
        tx.commit()?;
        Ok(Self {
            connection: Some(connection),
            path: path.into(),
            id,
            segment: None,
        })
    }
    pub fn event(&self, time: Timestamp, kind: &str, detail: &str) -> DataResult<()> {
        self.db()?.execute(
            "INSERT INTO event(utc_us,elapsed_us,kind,detail) VALUES(?,?,?,?)",
            params![time.utc_us, time.elapsed_us, kind, detail],
        )?;
        Ok(())
    }
    pub fn state(&self, status: &str, time: Timestamp) -> DataResult<()> {
        self.db()?.execute("UPDATE session SET status=?,started_utc_us=CASE WHEN ?='recording' AND started_utc_us IS NULL THEN ? ELSE started_utc_us END",params![status,status,time.utc_us])?;
        Ok(())
    }
    pub fn connect(&mut self, info: &SourceInfo, time: Timestamp) -> DataResult<()> {
        self.db()?.execute("INSERT INTO connection_segment(opened_utc_us,opened_elapsed_us,transport,adapter) VALUES(?,?,?,?)",params![time.utc_us,time.elapsed_us,info.transport,info.adapter])?;
        self.segment = Some(self.db()?.last_insert_rowid());
        Ok(())
    }
    pub fn disconnected(&self, time: Timestamp) -> DataResult<()> {
        self.db()?.execute(
            "UPDATE connection_segment SET closed_utc_us=? WHERE id=?",
            params![time.utc_us, self.segment],
        )?;
        Ok(())
    }
    pub fn begin_scan(
        &self,
        trigger: ScanTrigger,
        time: Timestamp,
        requested: &str,
    ) -> DataResult<i64> {
        self.db()?.execute("INSERT INTO dtc_scan_group(trigger,started_utc_us,started_elapsed_us,requested,coverage) VALUES(?,?,?,?,'attempted')",params![trigger.label(),time.utc_us,time.elapsed_us,requested])?;
        Ok(self.db()?.last_insert_rowid())
    }
    pub fn end_scan(&self, id: i64, time: Timestamp, complete: bool) -> DataResult<()> {
        self.db()?.execute(
            "UPDATE dtc_scan_group SET ended_utc_us=?,ended_elapsed_us=?,coverage=? WHERE id=?",
            params![
                time.utc_us,
                time.elapsed_us,
                if complete {
                    "complete_for_profile"
                } else {
                    "partial"
                },
                id
            ],
        )?;
        Ok(())
    }
    pub fn set_vin(&self, vin: &str) -> DataResult<()> {
        self.db()?
            .execute("UPDATE vehicle SET vin=?,vin_status='verified'", [vin])?;
        Ok(())
    }
    pub fn vin_conflict(&self) -> DataResult<()> {
        self.db()?
            .execute("UPDATE vehicle SET vin_status='conflict'", [])?;
        Ok(())
    }
    pub fn flush(&self, batch: &[Pending]) -> DataResult<()> {
        if batch.is_empty() {
            return Ok(());
        }
        // A reader that holds WAL pages must not allow unlimited growth.
        let mut wal = self.path.as_os_str().to_os_string();
        wal.push("-wal");
        let wal = PathBuf::from(wal);
        if std::fs::metadata(wal)
            .map(|m| m.len() > 16 * 1024 * 1024)
            .unwrap_or(false)
        {
            self.checkpoint()?;
        }
        let connection = self.db()?;
        let tx = connection.unchecked_transaction()?;
        for p in batch {
            let failed = p.observation.failed();
            let any_good = p
                .observation
                .responses
                .iter()
                .any(|e| !matches!(e.value, ObservationValue::Failure { .. }));
            tx.execute(
                "INSERT INTO acquisition VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    p.sequence,
                    self.segment,
                    p.request.kind(),
                    p.request.description(),
                    p.due,
                    p.start.utc_us,
                    p.start.elapsed_us,
                    p.end.utc_us,
                    p.end.elapsed_us,
                    if failed {
                        if any_good { "partial" } else { "failed" }
                    } else {
                        "ok"
                    },
                    p.observation.error,
                    p.observation.recovery,
                    p.target.map(sender_key)
                ],
            )?;
            tx.execute("INSERT INTO transaction_evidence VALUES(?,?,?,?,?,?,?,?)",params![p.sequence,if p.observation.partial{"operation_partial"}else if matches!(p.request,ReadRequest::Diagnostic(_)){"profile_response"}else{"ecu_transaction"},p.observation.raw_response,p.observation.attempted_request,p.observation.protocol.map(|v|v as u8),p.observation.context.as_deref().unwrap_or("source-defined observation; operation timestamps; encoded intended request may be unavailable"),p.observation.partial,p.truncated])?;
            let mut scan_written = false;
            for response in &p.observation.responses {
                let protocol = p.observation.protocol.map_or(-1, |v| v as i32);
                let sender = sender_key(response.sender);
                let segment = self.segment.ok_or_else(|| {
                    DataError::State("observation without connection segment".into())
                })?;
                tx.execute(
                    "INSERT OR IGNORE INTO ecu(segment_id,protocol,sender) VALUES(?,?,?)",
                    params![segment, protocol, sender],
                )?;
                let ecu: i64 = tx.query_row(
                    "SELECT id FROM ecu WHERE segment_id=? AND protocol=? AND sender=?",
                    params![segment, protocol, sender],
                    |r| r.get(0),
                )?;
                if p.target.is_some_and(|s| s != response.sender) {
                    continue;
                }
                match (&p.request, &response.value) {
                    (ReadRequest::Vin, ObservationValue::Vin(vin)) => {
                        tx.execute(
                            "INSERT INTO vin_observation(acquisition_id,ecu_id,vin) VALUES(?,?,?)",
                            params![p.sequence, ecu, vin],
                        )?;
                    }
                    (ReadRequest::Signal(signal), value) => {
                        let (number, blob) = match value {
                            ObservationValue::Number(v) => (Some(*v), None),
                            ObservationValue::Raw(b) => (None, Some(b)),
                            _ => continue,
                        };
                        tx.execute("INSERT INTO measurement(acquisition_id,ecu_id,signal_id,observed_utc_us,observed_elapsed_us,value_type,number_value,blob_value,unit) VALUES(?,?,?,?,?,?,?,?,?)",params![p.sequence,ecu,signal.id(),p.end.utc_us,p.end.elapsed_us,if number.is_some(){"number"}else{"blob"},number,blob,signal.unit()])?;
                    }
                    (ReadRequest::Supported(base), ObservationValue::Supported(bitmap)) => {
                        tx.execute(
                            "INSERT INTO capability_observation VALUES(?,?,?,?)",
                            params![p.sequence, ecu, base, *bitmap as i64],
                        )?;
                    }
                    (ReadRequest::Dtcs(_) | ReadRequest::Diagnostic(_), value) => {
                        let group = p
                            .scan_group
                            .ok_or_else(|| invalid("DTC observation has no group"))?;
                        let (category, namespace, profile) = match &p.request {
                            ReadRequest::Dtcs(k) => (
                                match k {
                                    crate::obd::DtcKind::Stored => "stored",
                                    crate::obd::DtcKind::Pending => "pending",
                                    crate::obd::DtcKind::Permanent => "permanent",
                                },
                                "obd2",
                                None,
                            ),
                            ReadRequest::Diagnostic(p) => (
                                "manufacturer_raw",
                                p.namespace.as_str(),
                                Some(p.id.as_str()),
                            ),
                            _ => unreachable!(),
                        };
                        let (outcome, count, raw, error) = match value {
                            ObservationValue::Dtcs(c) => ("ok", Some(c.len() as i64), None, None),
                            ObservationValue::Raw(b) => ("raw_only", None, Some(b), None),
                            ObservationValue::Failure { kind, message } => {
                                ("failed", None, None, Some(format!("{kind}: {message}")))
                            }
                            _ => return Err(invalid("invalid DTC result value")),
                        };
                        tx.execute("INSERT INTO dtc_scan_result(group_id,acquisition_id,ecu_id,category,namespace,profile_id,outcome,code_count,raw_payload,error) VALUES(?,?,?,?,?,?,?,?,?,?)",params![group,p.sequence,ecu,category,namespace,profile,outcome,count,raw,error])?;
                        let scan = tx.last_insert_rowid();
                        scan_written = true;
                        if let ObservationValue::Dtcs(codes) = value {
                            for (index, code) in codes.iter().enumerate() {
                                tx.execute("INSERT INTO dtc_entry(scan_id,ordinal,code,code_bytes,status_bytes) VALUES(?,?,?,?,?)",params![scan,index as i64,code.text,code.bytes,code.status])?;
                            }
                        }
                    }
                    _ => {}
                }
                if let ObservationValue::Failure { kind, message } = &response.value {
                    tx.execute("INSERT INTO event(utc_us,elapsed_us,kind,detail) VALUES(?,?,'ecu_failure',?)",params![p.end.utc_us,p.end.elapsed_us,format!("acquisition {} {sender}: {kind}: {message}",p.sequence)])?;
                }
            }
            if let Some(group) = p.scan_group {
                if !scan_written {
                    let (category, namespace, profile) = match &p.request {
                        ReadRequest::Dtcs(k) => (
                            match k {
                                crate::obd::DtcKind::Stored => "stored",
                                crate::obd::DtcKind::Pending => "pending",
                                crate::obd::DtcKind::Permanent => "permanent",
                            },
                            "obd2",
                            None,
                        ),
                        ReadRequest::Diagnostic(profile) => (
                            "manufacturer_raw",
                            profile.namespace.as_str(),
                            Some(profile.id.as_str()),
                        ),
                        _ => return Err(invalid("invalid scan request")),
                    };
                    tx.execute("INSERT INTO dtc_scan_result(group_id,acquisition_id,category,namespace,profile_id,outcome,error) VALUES(?,?,?,?,?,'failed',?)",params![group,p.sequence,category,namespace,profile,p.observation.error.as_deref().unwrap_or("no matching ECU result")])?;
                }
            }
            tx.execute("UPDATE session SET committed_sequence=?", [p.sequence])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn checkpoint(&self) -> DataResult<()> {
        let (busy, _, _): (i64, i64, i64) =
            self.db()?
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
        if busy != 0 {
            return Err(DataError::State(
                "WAL checkpoint blocked by a reader; release reader transactions".into(),
            ));
        }
        Ok(())
    }
    pub fn finalize(&mut self, time: Timestamp, ending_complete: bool) -> DataResult<()> {
        let valid: bool = self.db()?.query_row(
            "SELECT vin_status='verified' AND vin IS NOT NULL FROM vehicle",
            [],
            |r| r.get(0),
        )?;
        if !valid {
            return Err(invalid("cannot finalize without verified VIN"));
        }
        self.state("finalizing", time)?;
        self.checkpoint()?;
        let mode: String = self
            .db()?
            .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        if mode != "delete" {
            return Err(invalid("could not make recording standalone"));
        }
        self.db()?.pragma_update(None, "synchronous", "EXTRA")?;
        self.db()?.execute("UPDATE session SET status='completed',ended_utc_us=?,finalized_utc_us=?,ending_scan_complete=?",params![time.utc_us,time.utc_us,ending_complete])?;
        if let Some(connection) = self.connection.take() {
            connection.close().map_err(|(_, e)| DataError::Sqlite(e))?;
        }
        Ok(())
    }
    pub fn snapshot(&self, path: &Path, time: Timestamp) -> DataResult<()> {
        let mut destination = new_file(path)?;
        {
            let backup = rusqlite::backup::Backup::new(self.db()?, &mut destination)?;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match backup.step(64)? {
                    rusqlite::backup::StepResult::Done => break,
                    _ => {
                        if Instant::now() >= deadline {
                            return Err(DataError::State(
                                "snapshot backup deadline expired".into(),
                            ));
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            }
        }
        destination.pragma_update(None, "journal_mode", "DELETE")?;
        destination.pragma_update(None, "synchronous", "EXTRA")?;
        destination.execute("UPDATE session SET status='snapshot',exported_utc_us=?,export_cutoff_sequence=committed_sequence,ended_utc_us=NULL,finalized_utc_us=NULL",[time.utc_us])?;
        destination.close().map_err(|(_, e)| DataError::Sqlite(e))?;
        Ok(())
    }
    pub fn change_period(&self, index: usize, period_us: i64, time: Timestamp) -> DataResult<()> {
        let tx = self.db()?.unchecked_transaction()?;
        tx.execute(
            "UPDATE capture_selection SET period_us=? WHERE id=?",
            params![period_us, index as i64 + 1],
        )?;
        tx.execute("INSERT INTO selection_change(selection_id,utc_us,elapsed_us,period_us) VALUES(?,?,?,?)",params![index as i64+1,time.utc_us,time.elapsed_us,period_us])?;
        tx.execute(
            "INSERT INTO event(utc_us,elapsed_us,kind,detail) VALUES(?,?,'period_changed',?)",
            params![
                time.utc_us,
                time.elapsed_us,
                format!("selection {} period_us={period_us}", index + 1)
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn selection_capability(&self, index: usize, status: &str) -> DataResult<()> {
        self.db()?.execute(
            "UPDATE capture_selection SET capability=? WHERE id=?",
            params![status, index as i64 + 1],
        )?;
        Ok(())
    }
    pub fn vin(&self) -> DataResult<Option<String>> {
        Ok(self
            .db()?
            .query_row("SELECT vin FROM vehicle", [], |r| r.get(0))
            .optional()?
            .flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sqlite_full_rolls_back_acquisition_and_children() {
        let root = std::env::temp_dir().join(format!("kittx-sql-full-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("full.kittx");
        let _ = std::fs::remove_file(&path);
        {
            let config = RecordingConfig {
                signals: vec![SignalSelection::new(
                    Signal::RawPid(0x62),
                    Duration::from_secs(1),
                )],
                ..RecordingConfig::default()
            };
            let time = Timestamp {
                utc_us: 1,
                elapsed_us: 0,
            };
            let mut store = Store::create(&path, &config, time).unwrap();
            store
                .connect(
                    &SourceInfo {
                        transport: "synthetic".into(),
                        adapter: "ELM327 v2.1".into(),
                    },
                    time,
                )
                .unwrap();
            let pages: i64 = store
                .db()
                .unwrap()
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .unwrap();
            store
                .db()
                .unwrap()
                .pragma_update(None, "max_page_count", pages)
                .unwrap();
            let pending = Pending {
                sequence: 1,
                request: ReadRequest::Signal(Signal::RawPid(0x62)),
                due: 0,
                start: time,
                end: time,
                observation: Observation {
                    protocol: Some(crate::Protocol::Can11_500),
                    responses: vec![EcuObservation {
                        sender: crate::obd::Sender::Can(0x7E8),
                        value: ObservationValue::Raw(vec![0; 65536]),
                    }],
                    raw_response: Some(vec![0; 65536]),
                    ..Observation::default()
                },
                scan_group: None,
                target: None,
                truncated: false,
            };
            let error = store.flush(&[pending]).unwrap_err();
            assert!(
                matches!(error,DataError::Sqlite(rusqlite::Error::SqliteFailure(e,_)) if e.code==rusqlite::ErrorCode::DiskFull)
            );
            for table in ["acquisition", "measurement", "transaction_evidence"] {
                let n: i64 = store
                    .db()
                    .unwrap()
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap();
                assert_eq!(n, 0);
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
