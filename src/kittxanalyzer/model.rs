use kittx::datastore::{APPLICATION_ID, RecordingOverview, RecordingReader, SCHEMA_VERSION};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

/// One selectable DTC code or an empty, failed, or undecoded diagnostic read.
/// Codes retain category, namespace, profile, ECU, and read outcome identity.
#[derive(Debug, Clone)]
pub struct RecordedDtc {
    pub code: Option<String>,
    pub category: String,
    pub namespace: String,
    pub profile_id: Option<String>,
    pub ecu_id: Option<i64>,
    pub sender: Option<String>,
    pub outcome: String,
    pub reported_codes: Option<i64>,
    /// Number of distinct diagnostic reads, not the number of fault onsets.
    pub observations: i64,
    /// Acquisition completion times in monotonic elapsed microseconds.
    pub first_us: i64,
    pub last_us: i64,
}

impl RecordedDtc {
    /// Human-readable identity; unknown manufacturer codes are never interpreted.
    pub fn label(&self) -> String {
        let code = self.code.as_deref().unwrap_or(match self.outcome.as_str() {
            "failed" => "Failed DTC reads",
            "raw_only" => "Raw diagnostic replies",
            _ if self.reported_codes == Some(0) => "Empty DTC reads",
            _ => "No decoded DTC entries",
        });
        let category = match self.category.as_str() {
            "stored" => "Stored",
            "pending" => "Pending",
            "permanent" => "Permanent",
            other => other,
        };
        let ecu = self.ecu_id.map_or_else(|| "-".into(), |id| id.to_string());
        let mut label = format!("{code} · {category} · {} · ECU {ecu}", self.namespace);
        if let Some(sender) = &self.sender {
            label += &format!(" ({sender})");
        }
        if let Some(profile) = &self.profile_id {
            label += &format!(" · profile {profile}");
        }
        label
    }
}

/// A metadata-only snapshot. Original sample streams stay in the source file.
#[derive(Debug, Clone)]
pub struct Recording {
    pub path: PathBuf,
    pub overview: RecordingOverview,
    pub dtcs: Vec<RecordedDtc>,
}

/// Indices into a single recording's catalog. Reset when another file is loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub series: BTreeSet<usize>,
    pub dtcs: BTreeSet<usize>,
}

impl Selection {
    /// Select every recorded measurement series and diagnostic catalog item.
    pub fn all(recording: &Recording) -> Self {
        Self {
            series: (0..recording.overview.series.len()).collect(),
            dtcs: (0..recording.dtcs.len()).collect(),
        }
    }
}

impl Recording {
    /// Validate the existing format and read its catalog without modifying it.
    /// DTC summaries use the overview's committed acquisition cutoff, including
    /// empty/failed/raw reads. No samples or raw reply bodies are loaded.
    pub fn open(path: PathBuf) -> Result<Self, String> {
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("kittx"))
        {
            return Err("Choose a recording with the .kittx extension".into());
        }
        let overview = RecordingReader::open(&path)
            .and_then(|reader| reader.overview())
            .map_err(|error| error.to_string())?;
        let connection = snapshot_connection(&path, &overview)?;
        let dtcs = connection
            .prepare(
                "SELECT entry.code,scan.category,scan.namespace,scan.profile_id,
                        scan.ecu_id,ecu.sender,scan.outcome,MAX(scan.code_count),
                        COUNT(DISTINCT scan.id),MIN(a.operation_completed_elapsed_us),
                        MAX(a.operation_completed_elapsed_us)
                 FROM dtc_scan_result scan
                 JOIN acquisition a ON a.id=scan.acquisition_id
                 LEFT JOIN dtc_entry entry ON entry.scan_id=scan.id
                 LEFT JOIN ecu ON ecu.id=scan.ecu_id
                 WHERE scan.acquisition_id<=?1
                 GROUP BY entry.code,scan.category,scan.namespace,scan.profile_id,
                          scan.ecu_id,scan.outcome,(scan.code_count=0)
                 ORDER BY scan.category,scan.namespace,scan.profile_id,scan.ecu_id,entry.code,scan.outcome",
            )
            .map_err(|error| error.to_string())?
            .query_map([overview.session.committed_sequence], |row| {
                Ok(RecordedDtc {
                    code: row.get(0)?,
                    category: row.get(1)?,
                    namespace: row.get(2)?,
                    profile_id: row.get(3)?,
                    ecu_id: row.get(4)?,
                    sender: row.get(5)?,
                    outcome: row.get(6)?,
                    reported_codes: row.get(7)?,
                    observations: row.get(8)?,
                    first_us: row.get(9)?,
                    last_us: row.get(10)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            path,
            overview,
            dtcs,
        })
    }

    /// Preview precisely the user's selection. This does not analyze, transmit,
    /// decode, or write recording data and produces no diagnostic conclusions.
    pub fn selection_preview(&self, selection: &Selection) -> Result<String, String> {
        if selection.series.is_empty() && selection.dtcs.is_empty() {
            return Err("Select at least one measurement or DTC item".into());
        }
        let mut report = format!(
            "Selection ready\n\nLLM analysis is not connected yet. This is a selection preview; no analysis has run.\n\nFile: {}\nSession: {}\nVIN: {}\nCommitted acquisition cutoff: {}\n\nMeasurements ({})\n",
            self.path.display(),
            self.overview.session.id,
            self.overview.session.vin.as_deref().unwrap_or("-"),
            self.overview.session.committed_sequence,
            selection.series.len(),
        );
        for &index in &selection.series {
            let series = self
                .overview
                .series
                .get(index)
                .ok_or("Unknown measurement selection")?;
            report += &format!(
                "• {} [{}] · ECU {} ({}) · {} · {} samples{}\n",
                signal_label(&series.signal),
                series.signal,
                series.ecu_id,
                series.sender,
                series.unit,
                series.count,
                if series.numeric {
                    ""
                } else {
                    " · raw / undecoded"
                },
            );
        }
        report += &format!("\nDTC items ({})\n", selection.dtcs.len());
        for &index in &selection.dtcs {
            let dtc = self.dtcs.get(index).ok_or("Unknown DTC selection")?;
            let first =
                (i128::from(dtc.first_us) - i128::from(self.overview.origin_us)) as f64 / 1e6;
            let last = (i128::from(dtc.last_us) - i128::from(self.overview.origin_us)) as f64 / 1e6;
            report += &format!(
                "• {} · {} reads · observed {:.6}–{:.6} s from recording start · result {}\n",
                dtc.label(),
                dtc.observations,
                first,
                last,
                dtc.outcome,
            );
        }
        Ok(report)
    }
}

/// Keep a consistent read-only transaction until the returned connection drops.
pub(crate) fn snapshot_connection(
    path: &Path,
    overview: &RecordingOverview,
) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_millis(250))
        .map_err(|error| error.to_string())?;
    connection
        .pragma_update(None, "trusted_schema", false)
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch("BEGIN")
        .map_err(|error| error.to_string())?;
    let (application, version): (i32, i32) = connection
        .query_row(
            "SELECT (SELECT application_id FROM pragma_application_id),
                (SELECT user_version FROM pragma_user_version)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let (id, cutoff): (String, i64) = connection
        .query_row("SELECT id,committed_sequence FROM session", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| error.to_string())?;
    if application != APPLICATION_ID
        || version != SCHEMA_VERSION
        || id != overview.session.id
        || cutoff < overview.session.committed_sequence
    {
        return Err("The recording changed while opening; reopen it".into());
    }
    Ok(connection)
}

/// Known decoded measurements; unfamiliar signal IDs remain visible verbatim.
pub fn signal_label(id: &str) -> &str {
    match id {
        "01:04" => "Engine load",
        "01:05" => "Coolant temperature",
        "01:0B" => "Manifold pressure",
        "01:0C" => "Engine RPM",
        "01:0D" => "Vehicle speed",
        "01:0F" => "Intake temperature",
        "01:11" => "Throttle position",
        "adapter:voltage" => "Adapter battery voltage",
        other => other,
    }
}
