//! Viewer-only projections over read-only raw measurements. No migrations/caches
//! are written to the source recording; the frozen acquisition cutoff is mandatory.
use crate::pid;
use kittx::datastore::{PlotSample, RecordedSeries, RecordingOverview};
use rusqlite::{Connection, OpenFlags, functions::FunctionFlags, params};
use std::path::Path;

#[derive(Clone)]
pub(crate) struct DerivedSeries {
    pub source: RecordedSeries,
    pub field: usize,
}

pub(crate) struct PidReader {
    connection: Connection,
    failures: bool,
}
impl PidReader {
    pub fn open(path: &Path, session: &str) -> Result<Self, String> {
        let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        c.pragma_update(None, "query_only", true)
            .map_err(|e| e.to_string())?;
        let id: String = c
            .query_row("SELECT id FROM session", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if id != session {
            return Err("The recording file was replaced; reopen it".into());
        }
        c.create_scalar_function(
            "pid_value",
            2,
            FunctionFlags::SQLITE_UTF8
                | FunctionFlags::SQLITE_DETERMINISTIC
                | FunctionFlags::SQLITE_INNOCUOUS,
            |ctx| {
                let index = ctx.get::<usize>(0)?;
                let bytes = match ctx.get_raw(1) {
                    rusqlite::types::ValueRef::Blob(bytes) => Some(bytes),
                    _ => None,
                };
                Ok(pid::fields()
                    .get(index)
                    .and_then(|f| bytes.and_then(|b| f.decode(b))))
            },
        )
        .map_err(|e| e.to_string())?;
        let failures = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM acquisition WHERE outcome!='ok')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            connection: c,
            failures,
        })
    }

    pub fn extend_overview(
        &self,
        overview: &mut RecordingOverview,
    ) -> Result<Vec<Option<DerivedSeries>>, String> {
        let mut derived = vec![None; overview.series.len()];
        let mut additions = Vec::new();
        let mut query = self
            .connection
            .prepare_cached(
                "SELECT COUNT(*),MIN(m.observed_elapsed_us),MAX(m.observed_elapsed_us)
             FROM measurement m WHERE m.ecu_id=?1 AND m.signal_id=?2 AND m.unit=?3
             AND m.acquisition_id<=?4 AND m.value_type='blob'
             AND pid_value(?5,m.blob_value) IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        for source in &overview.series {
            if source.numeric {
                continue;
            }
            for (index, field) in pid::fields()
                .iter()
                .enumerate()
                .filter(|(_, f)| f.source_signal() == source.signal)
            {
                let (count, first, last): (i64, Option<i64>, Option<i64>) = query
                    .query_row(
                        params![
                            source.ecu_id,
                            source.signal,
                            source.unit,
                            overview.session.committed_sequence,
                            index
                        ],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .map_err(|e| e.to_string())?;
                if let (Some(first_us), Some(last_us)) = (first, last) {
                    additions.push(RecordedSeries {
                        signal: field.signal(),
                        unit: field.unit.into(),
                        numeric: true,
                        count,
                        first_us,
                        last_us,
                        ..source.clone()
                    });
                    derived.push(Some(DerivedSeries {
                        source: source.clone(),
                        field: index,
                    }));
                }
            }
        }
        overview.series.extend(additions);
        Ok(derived)
    }

    pub fn plot(
        &self,
        series: &DerivedSeries,
        cutoff: i64,
        from: i64,
        to: i64,
        buckets: u32,
    ) -> Result<Vec<PlotSample>, String> {
        if from > to || !(1..=4096).contains(&buckets) {
            return Err("Invalid plot bounds/bucket count".into());
        }
        let source = &series.source;
        let sql = r"WITH periods AS MATERIALIZED (
 SELECT c.id,c.elapsed_us,c.period_us FROM selection_change c JOIN capture_selection cs ON cs.id=c.selection_id WHERE cs.signal_id=?2
), initial_periods AS MATERIALIZED (
 SELECT enabled_elapsed_us,period_us FROM capture_selection WHERE signal_id=?2
), decoded AS (
 SELECT m.id,m.acquisition_id,m.observed_elapsed_us,m.observed_utc_us,
 pid_value(?8,m.blob_value) number_value,
 a.operation_started_elapsed_us started,a.operation_completed_elapsed_us completed,a.outcome!='ok' partial,a.request,
 COALESCE((SELECT c.period_us FROM periods c WHERE c.elapsed_us<=m.observed_elapsed_us ORDER BY c.elapsed_us DESC,c.id DESC LIMIT 1),(SELECT period_us FROM initial_periods cs WHERE cs.enabled_elapsed_us<=m.observed_elapsed_us ORDER BY cs.enabled_elapsed_us DESC LIMIT 1),1000000) period
 FROM measurement m JOIN acquisition a ON a.id=m.acquisition_id
 WHERE m.ecu_id=?1 AND m.signal_id=?2 AND m.unit=?7 AND m.acquisition_id<=?6 AND m.value_type='blob'
), original AS (
 SELECT *,LAG(observed_elapsed_us) OVER(ORDER BY observed_elapsed_us,id) previous_time,
 LAG(acquisition_id) OVER(ORDER BY observed_elapsed_us,id) previous_acquisition,
 LAG(number_value) OVER(ORDER BY observed_elapsed_us,id) previous_value FROM decoded
), all_runs AS (
 SELECT *,SUM(CASE WHEN previous_value IS NULL OR number_value IS NULL OR observed_elapsed_us-previous_time>3*period OR partial OR (?9 AND EXISTS(SELECT 1 FROM acquisition failed WHERE failed.id>original.previous_acquisition AND failed.id<=original.acquisition_id AND failed.request=original.request AND failed.outcome!='ok')) THEN 1 ELSE 0 END) OVER(ORDER BY observed_elapsed_us,id) run_id FROM original
), runs AS (
 SELECT * FROM all_runs WHERE number_value IS NOT NULL
), visible AS (
 SELECT *,MIN(?5-1,CAST((observed_elapsed_us-?3)*1.0/MAX(1,?4-?3)*?5 AS INTEGER)) bucket FROM runs WHERE observed_elapsed_us BETWEEN ?3 AND ?4
), ranked AS (
 SELECT *,ROW_NUMBER() OVER(PARTITION BY bucket ORDER BY observed_elapsed_us,id) f,ROW_NUMBER() OVER(PARTITION BY bucket ORDER BY observed_elapsed_us DESC,id DESC) l,ROW_NUMBER() OVER(PARTITION BY bucket ORDER BY number_value,id) lo,ROW_NUMBER() OVER(PARTITION BY bucket ORDER BY number_value DESC,id) hi FROM visible
), chosen AS (
 SELECT id FROM ranked WHERE f=1 OR l=1 OR lo=1 OR hi=1
 UNION SELECT (SELECT id FROM runs WHERE observed_elapsed_us<?3 ORDER BY observed_elapsed_us DESC,id DESC LIMIT 1)
 UNION SELECT (SELECT id FROM runs WHERE observed_elapsed_us>?4 ORDER BY observed_elapsed_us,id LIMIT 1)
) SELECT m.id,m.acquisition_id,m.observed_elapsed_us,m.observed_utc_us,m.number_value,m.started,m.completed,m.partial,m.run_id FROM runs m JOIN chosen c ON c.id=m.id ORDER BY m.observed_elapsed_us,m.id";
        self.connection
            .prepare_cached(sql)
            .map_err(|e| e.to_string())?
            .query_map(
                params![
                    source.ecu_id,
                    source.signal,
                    from,
                    to,
                    buckets,
                    cutoff,
                    source.unit,
                    series.field,
                    self.failures
                ],
                sample,
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn nearest(
        &self,
        series: &DerivedSeries,
        cutoff: i64,
        at: i64,
    ) -> Result<Option<PlotSample>, String> {
        let source = &series.source;
        // Search only as far as the nearest valid original sample on each side.
        // Malformed/unsupported values are skipped without a display-resolution limit.
        let mut points = Vec::new();
        for before in [true, false] {
            let (comparison, order) = if before {
                ("<=", "DESC")
            } else {
                (">=", "ASC")
            };
            let sql = format!(
                "SELECT m.id,m.acquisition_id,m.observed_elapsed_us,m.observed_utc_us,
                 pid_value(?6,m.blob_value),a.operation_started_elapsed_us,a.operation_completed_elapsed_us,a.outcome!='ok',0
                 FROM measurement m JOIN acquisition a ON a.id=m.acquisition_id
                 WHERE m.ecu_id=?1 AND m.signal_id=?2 AND m.unit=?3 AND m.acquisition_id<=?4
                 AND m.value_type='blob' AND m.observed_elapsed_us {comparison} ?5
                 AND pid_value(?6,m.blob_value) IS NOT NULL
                 ORDER BY m.observed_elapsed_us {order},m.id {order} LIMIT 1"
            );
            points.extend(
                self.connection
                    .prepare_cached(&sql)
                    .map_err(|e| e.to_string())?
                    .query_map(
                        params![
                            source.ecu_id,
                            source.signal,
                            source.unit,
                            cutoff,
                            at,
                            series.field
                        ],
                        sample,
                    )
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(points
            .into_iter()
            .min_by_key(|p| (p.elapsed_us.abs_diff(at), p.id)))
    }
}

fn sample(r: &rusqlite::Row<'_>) -> rusqlite::Result<PlotSample> {
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
}
