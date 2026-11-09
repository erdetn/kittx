//! Bounded, read-only queries for recording viewers. Times are integer microseconds.
use super::*;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct RecordedSeries {
    pub ecu_id: i64,
    pub segment_id: i64,
    pub protocol: i32,
    pub sender: String,
    pub signal: String,
    pub unit: String,
    pub numeric: bool,
    pub count: i64,
    pub first_us: i64,
    pub last_us: i64,
    pub period_us: Option<i64>,
}
#[derive(Debug, Clone)]
pub struct PlotSample {
    pub id: i64,
    pub acquisition_id: i64,
    pub elapsed_us: i64,
    pub utc_us: i64,
    pub value: f64,
    pub started_us: i64,
    pub completed_us: i64,
    pub partial: bool,
    /// Contiguous original-sample run in plot queries; zero for isolated nearest lookups.
    pub run_id: i64,
}
#[derive(Debug, Clone)]
pub struct RecordingOverview {
    pub session: SessionInfo,
    pub vehicle: Vec<VehicleField>,
    pub series: Vec<RecordedSeries>,
    pub first_us: i64,
    pub last_us: i64,
    pub origin_us: i64,
    pub producer: String,
    pub connections: Vec<String>,
    pub dtc_occurrences: i64,
    pub unique_codes: i64,
    /// Recording policy and bounded sampling-change history, including timestamps.
    pub policy: Vec<String>,
}
impl RecordingReader {
    /// Acquire a short consistent metadata snapshot. Subsequent queries must use its cutoff.
    pub fn overview(&self) -> DataResult<RecordingOverview> {
        self.connection.execute_batch("BEGIN")?;
        let result = (|| {
            let session = self.session()?;
            let cutoff = session.committed_sequence;
            let mut stmt = self.connection.prepare("SELECT e.id,e.segment_id,e.protocol,e.sender,m.signal_id,m.unit,MAX(m.value_type='number'),COUNT(*),MIN(m.observed_elapsed_us),MAX(m.observed_elapsed_us),(SELECT MIN(period_us) FROM capture_selection WHERE signal_id=m.signal_id) FROM measurement m JOIN ecu e ON e.id=m.ecu_id WHERE m.acquisition_id<=? GROUP BY e.id,m.signal_id,m.unit ORDER BY m.signal_id,e.id")?;
            let series = stmt
                .query_map([cutoff], |r| {
                    Ok(RecordedSeries {
                        ecu_id: r.get(0)?,
                        segment_id: r.get(1)?,
                        protocol: r.get(2)?,
                        sender: r.get(3)?,
                        signal: r.get(4)?,
                        unit: r.get(5)?,
                        numeric: r.get(6)?,
                        count: r.get(7)?,
                        first_us: r.get(8)?,
                        last_us: r.get(9)?,
                        period_us: r.get(10)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let (first_us,last_us) = self.connection.query_row("SELECT COALESCE(MIN(operation_completed_elapsed_us),0),COALESCE(MAX(operation_completed_elapsed_us),0) FROM acquisition WHERE id<=?",[cutoff],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let origin_us = self.connection.query_row(
                "SELECT COALESCE((SELECT started_utc_us-created_utc_us FROM session),0)",
                [],
                |r| r.get(0),
            )?;
            let producer = self.connection.query_row("SELECT producer||' '||producer_version||' / library '||library_version||' / '||platform FROM session",[],|r|r.get(0))?;
            let connections = self.connection.prepare("SELECT 'Segment '||id||': '||transport||' / '||adapter FROM connection_segment WHERE opened_elapsed_us<=? ORDER BY id")?.query_map([last_us],|r|r.get(0))?.collect::<Result<Vec<_>,_>>()?;
            let (dtc_occurrences,unique_codes) = self.connection.query_row("SELECT COUNT(*),COUNT(DISTINCT s.namespace||':'||d.code) FROM dtc_entry d JOIN dtc_scan_result s ON s.id=d.scan_id WHERE s.acquisition_id<=?",[cutoff],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let mut policy=self.connection.prepare("SELECT 'Operation timeout: '||operation_timeout_us||' µs; commit: '||commit_mode||'; DTC period: '||COALESCE(dtc_period_us||' µs','-') FROM recording_policy")?.query_map([],|r|r.get(0))?.collect::<Result<Vec<String>,_>>()?;
            let changes=self.connection.prepare("SELECT 'Signal '||s.signal_id||' period '||c.period_us||' µs at elapsed '||c.elapsed_us||' µs' FROM selection_change c JOIN capture_selection s ON s.id=c.selection_id WHERE c.elapsed_us<=? ORDER BY c.elapsed_us,c.id LIMIT 1001")?.query_map([last_us],|r|r.get(0))?.collect::<Result<Vec<String>,_>>()?;
            policy.extend(changes.into_iter().take(1000));
            Ok(RecordingOverview {
                session,
                vehicle: self.vehicle_fields()?,
                series,
                first_us,
                last_us,
                origin_us,
                producer,
                connections,
                dtc_occurrences,
                unique_codes,
                policy,
            })
        })();
        self.connection.execute_batch("ROLLBACK")?;
        result
    }
    /// Original first/last/min/max samples per horizontal bucket, ordered by time.
    /// At most 4 * buckets + 2 rows; includes immediate viewport neighbors.
    pub fn plot_samples(
        &self,
        series: &RecordedSeries,
        cutoff: i64,
        from: i64,
        to: i64,
        buckets: u32,
    ) -> DataResult<Vec<PlotSample>> {
        if from > to || !(1..=4096).contains(&buckets) {
            return Err(invalid("invalid plot bounds/bucket count"));
        }
        let sql = r"WITH original AS (
 SELECT m.*,a.operation_started_elapsed_us started,a.operation_completed_elapsed_us completed,a.outcome!='ok' partial,a.request,
 LAG(m.observed_elapsed_us) OVER(ORDER BY m.observed_elapsed_us,m.id) previous_time,
 LAG(m.acquisition_id) OVER(ORDER BY m.observed_elapsed_us,m.id) previous_acquisition,
 COALESCE((SELECT c.period_us FROM selection_change c JOIN capture_selection cs ON cs.id=c.selection_id WHERE cs.signal_id=m.signal_id AND c.elapsed_us<=m.observed_elapsed_us ORDER BY c.elapsed_us DESC,c.id DESC LIMIT 1),(SELECT period_us FROM capture_selection cs WHERE cs.signal_id=m.signal_id AND cs.enabled_elapsed_us<=m.observed_elapsed_us ORDER BY cs.enabled_elapsed_us DESC LIMIT 1),1000000) period
 FROM measurement m JOIN acquisition a ON a.id=m.acquisition_id
 WHERE m.ecu_id=?1 AND m.signal_id=?2 AND m.unit=?7 AND m.acquisition_id<=?6 AND m.value_type='number'
), runs AS (
 SELECT *,SUM(CASE WHEN previous_time IS NULL OR observed_elapsed_us-previous_time>3*period OR partial OR EXISTS(SELECT 1 FROM acquisition failed WHERE failed.id>original.previous_acquisition AND failed.id<=original.acquisition_id AND failed.request=original.request AND failed.outcome!='ok') THEN 1 ELSE 0 END) OVER(ORDER BY observed_elapsed_us,id) run_id FROM original
), visible AS (
 SELECT *,MIN(?5-1,CAST((observed_elapsed_us-?3)*1.0/MAX(1,?4-?3)*?5 AS INTEGER)) bucket FROM runs WHERE observed_elapsed_us BETWEEN ?3 AND ?4
), ranked AS (
 SELECT *,ROW_NUMBER() OVER(PARTITION BY bucket ORDER BY observed_elapsed_us,id) f,ROW_NUMBER() OVER(PARTITION BY bucket ORDER BY observed_elapsed_us DESC,id DESC) l,ROW_NUMBER() OVER(PARTITION BY bucket ORDER BY number_value,id) lo,ROW_NUMBER() OVER(PARTITION BY bucket ORDER BY number_value DESC,id) hi FROM visible
), chosen AS (
 SELECT id FROM ranked WHERE f=1 OR l=1 OR lo=1 OR hi=1
 UNION SELECT (SELECT id FROM runs WHERE observed_elapsed_us<?3 ORDER BY observed_elapsed_us DESC,id DESC LIMIT 1)
 UNION SELECT (SELECT id FROM runs WHERE observed_elapsed_us>?4 ORDER BY observed_elapsed_us,id LIMIT 1)
) SELECT m.id,m.acquisition_id,m.observed_elapsed_us,m.observed_utc_us,m.number_value,m.started,m.completed,m.partial,m.run_id FROM runs m JOIN chosen c ON c.id=m.id ORDER BY m.observed_elapsed_us,m.id";
        Ok(self
            .connection
            .prepare(sql)?
            .query_map(
                params![
                    series.ecu_id,
                    series.signal,
                    from,
                    to,
                    buckets,
                    cutoff,
                    series.unit
                ],
                |r| {
                    Ok(PlotSample {
                        id: r.get(0)?,
                        acquisition_id: r.get(1)?,
                        elapsed_us: r.get(2)?,
                        utc_us: r.get(3)?,
                        value: r.get(4)?,
                        started_us: r.get(5)?,
                        completed_us: r.get(6)?,
                        partial: r.get(7)?,
                        run_id: r.get(8)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?)
    }
    /// Nearest original numeric sample, independent of rendered envelope resolution.
    pub fn nearest_sample(
        &self,
        series: &RecordedSeries,
        cutoff: i64,
        at: i64,
    ) -> DataResult<Option<PlotSample>> {
        let sql = r"WITH candidates AS (
        SELECT (SELECT id FROM measurement WHERE ecu_id=?1 AND signal_id=?2 AND unit=?3 AND acquisition_id<=?4 AND value_type='number' AND observed_elapsed_us<=?5 ORDER BY observed_elapsed_us DESC,id DESC LIMIT 1) id
        UNION SELECT (SELECT id FROM measurement WHERE ecu_id=?1 AND signal_id=?2 AND unit=?3 AND acquisition_id<=?4 AND value_type='number' AND observed_elapsed_us>=?5 ORDER BY observed_elapsed_us,id LIMIT 1)
        ) SELECT m.id,m.acquisition_id,m.observed_elapsed_us,m.observed_utc_us,m.number_value,a.operation_started_elapsed_us,a.operation_completed_elapsed_us,a.outcome!='ok' FROM measurement m JOIN candidates c ON c.id=m.id JOIN acquisition a ON a.id=m.acquisition_id";
        let points = self
            .connection
            .prepare(sql)?
            .query_map(
                params![series.ecu_id, series.signal, series.unit, cutoff, at],
                |r| {
                    Ok(PlotSample {
                        id: r.get(0)?,
                        acquisition_id: r.get(1)?,
                        elapsed_us: r.get(2)?,
                        utc_us: r.get(3)?,
                        value: r.get(4)?,
                        started_us: r.get(5)?,
                        completed_us: r.get(6)?,
                        partial: r.get(7)?,
                        run_id: 0,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(points
            .into_iter()
            .filter(|p| p.value.is_finite())
            .min_by_key(|p| p.elapsed_us.abs_diff(at)))
    }
}

/// DTC observation projection; failures never establish an empty code set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DtcDisplayMode {
    #[default]
    All,
    FirstObserved,
    SetChanges,
}

/// Lightweight clustered scan history. Raw evidence is fetched separately on selection.
#[derive(Debug, Clone)]
pub struct DtcMarker {
    pub scan: DtcScanRow,
    pub scans: i64,
    pub last_us: i64,
    pub bucket_start_us: i64,
    pub bucket_end_us: i64,
}
impl RecordingReader {
    /// Cluster every scan into bounded time buckets, retaining occurrence counts.
    /// Scan timing always uses acquisition completion, including failed/empty scans.
    pub fn dtc_markers(
        &self,
        cutoff: i64,
        from: i64,
        to: i64,
        buckets: u32,
    ) -> DataResult<Vec<DtcMarker>> {
        self.dtc_markers_mode(cutoff, from, to, buckets, DtcDisplayMode::All)
    }
    /// Optional first-code observation or complete-successful-set changes across full history.
    pub fn dtc_markers_mode(
        &self,
        cutoff: i64,
        from: i64,
        to: i64,
        buckets: u32,
        mode: DtcDisplayMode,
    ) -> DataResult<Vec<DtcMarker>> {
        if from > to || !(1..=4096).contains(&buckets) {
            return Err(invalid("invalid DTC viewport"));
        }
        let sql = r"WITH sets AS (
        SELECT d.id,d.group_id,d.acquisition_id,d.ecu_id,d.category,d.namespace,d.profile_id,d.outcome,d.code_count,d.error,a.operation_completed_elapsed_us elapsed,a.operation_completed_utc_us utc,
        COALESCE((SELECT group_concat(code,'|') FROM (SELECT code FROM dtc_entry WHERE scan_id=d.id ORDER BY code)),'') codes
        FROM dtc_scan_result d JOIN acquisition a ON a.id=d.acquisition_id WHERE d.acquisition_id<=?1
        ), successful AS (
        SELECT *,LAG(codes) OVER(PARTITION BY ecu_id,category,namespace ORDER BY elapsed,id) previous_codes FROM sets WHERE outcome='ok'
        ), projected AS (
        SELECT id FROM successful d WHERE ?5=0 OR (?5=2 AND (previous_codes IS NULL OR previous_codes!=codes)) OR (?5=1 AND EXISTS(
        SELECT 1 FROM dtc_entry entry WHERE entry.scan_id=d.id AND NOT EXISTS(
        SELECT 1 FROM dtc_entry old JOIN dtc_scan_result prior ON prior.id=old.scan_id JOIN acquisition pa ON pa.id=prior.acquisition_id
        WHERE old.code=entry.code AND prior.outcome='ok' AND prior.ecu_id IS d.ecu_id AND prior.category=d.category AND prior.namespace=d.namespace AND prior.acquisition_id<=?1 AND (pa.operation_completed_elapsed_us<d.elapsed OR (pa.operation_completed_elapsed_us=d.elapsed AND prior.id<d.id)))))
        UNION ALL SELECT id FROM sets WHERE outcome!='ok'
        ), scans AS (
        SELECT d.*,
        MIN(?4-1,CAST((d.elapsed-?2)*1.0/MAX(1,?3-?2)*?4 AS INTEGER)) bucket
        FROM sets d JOIN projected p ON p.id=d.id
        WHERE d.elapsed BETWEEN ?2 AND ?3
        ), clustered AS (
        SELECT *,COUNT(*) OVER w members,MAX(elapsed) OVER w last_elapsed,
        ROW_NUMBER() OVER(PARTITION BY bucket,ecu_id,category,namespace,outcome,code_count>0 ORDER BY elapsed,id) ordinal
        FROM scans WINDOW w AS(PARTITION BY bucket,ecu_id,category,namespace,outcome,code_count>0)
        ) SELECT id,group_id,acquisition_id,ecu_id,category,namespace,profile_id,outcome,code_count,error,utc,elapsed,members,last_elapsed,bucket FROM clustered WHERE ordinal=1 ORDER BY elapsed,id LIMIT 20001";
        let rows = self
            .connection
            .prepare(sql)?
            .query_map(
                params![
                    cutoff,
                    from,
                    to,
                    buckets,
                    match mode {
                        DtcDisplayMode::All => 0,
                        DtcDisplayMode::FirstObserved => 1,
                        DtcDisplayMode::SetChanges => 2,
                    }
                ],
                |r| {
                    let bucket: i64 = r.get(14)?;
                    let span = (to - from).max(1);
                    Ok(DtcMarker {
                        scan: DtcScanRow {
                            id: r.get(0)?,
                            group_id: r.get(1)?,
                            acquisition_id: r.get(2)?,
                            ecu_id: r.get(3)?,
                            category: r.get(4)?,
                            namespace: r.get(5)?,
                            profile_id: r.get(6)?,
                            outcome: r.get(7)?,
                            code_count: r.get(8)?,
                            raw_payload: None,
                            error: r.get(9)?,
                            utc_us: r.get(10)?,
                            elapsed_us: r.get(11)?,
                        },
                        scans: r.get(12)?,
                        last_us: r.get(13)?,
                        bucket_start_us: from + span * bucket / i64::from(buckets),
                        bucket_end_us: from + span * (bucket + 1) / i64::from(buckets),
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.len() > 20000 {
            return Err(invalid("too many DTC groups; zoom in"));
        }
        Ok(rows)
    }
}
