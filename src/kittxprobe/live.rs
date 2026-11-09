//! Frozen read-only projections of committed data. No ECU requests or file writes.
use kittx::datastore::{DtcScanRow, PlotSample, Query, RecordingReader};
use kittx_viewer::{
    document::Document,
    model::{Viewport, signal_label},
};
use rusqlite::{Connection, OpenFlags, params};
use std::{collections::BTreeMap, path::Path};

/// A separately identified ECU/connection/signal curve; values use `UnitTrack::unit`.
#[derive(Clone)]
pub struct SignalPlot {
    pub signal: String,
    pub label: String,
    pub sender: String,
    pub segment: i64,
    pub points: Vec<PlotSample>,
}

/// All curves with the exact same engineering unit share a Y scale.
#[derive(Clone)]
pub struct UnitTrack {
    pub unit: String,
    pub signals: Vec<SignalPlot>,
}

/// One actual read, including successful empty, raw, failed and unsupported reads.
#[derive(Clone)]
pub struct DtcRead {
    pub scan: DtcScanRow,
    pub sender: String,
    pub codes: Vec<String>,
}

/// The latest committed snapshot. Automatic live plots show the last 60 seconds;
/// after Stop, they show the whole interval. An explicit view overrides this.
/// DTC history keeps the latest 200 reads across the session, independently of
/// the chosen plot window.
#[derive(Clone)]
pub struct LiveSnapshot {
    /// Whole committed interval available for zooming and panning.
    pub full: Viewport,
    pub view: Viewport,
    pub origin_us: i64,
    pub tracks: Vec<UnitTrack>,
    pub dtcs: Vec<DtcRead>,
    pub dtc_history_limited: bool,
    pub raw_series: usize,
    pub cutoff: i64,
}

impl LiveSnapshot {
    /// Reads and releases short SQLite snapshots. Existing ViewerUI decoders
    /// interpret all supported numeric raw-PID fields; source bytes stay intact.
    pub fn read(path: &Path, rolling: bool, pixels: u32) -> Result<Self, String> {
        Self::read_view(path, rolling, pixels, None)
    }

    /// Reloads samples at the requested time scale, bounded to the committed
    /// interval. `None` restores the automatic rolling/full view. DTC history
    /// remains session-wide. This performs no writes or ECU requests.
    pub fn read_view(
        path: &Path,
        rolling: bool,
        pixels: u32,
        requested: Option<Viewport>,
    ) -> Result<Self, String> {
        let doc = Document::open(path.to_owned())?;
        let full = doc.full();
        let view = if let Some(view) = requested {
            bounded_view(view, full)
        } else if rolling {
            Viewport::new(full.from.max(full.to.saturating_sub(60_000_000)), full.to)
        } else {
            full
        };
        let selected: Vec<_> = doc
            .overview
            .series
            .iter()
            .enumerate()
            .filter(|(_, s)| s.numeric)
            .map(|(i, _)| i)
            .collect();
        let frame = doc.frame(view, &selected, pixels, Default::default())?;
        let mut tracks: BTreeMap<String, Vec<SignalPlot>> = BTreeMap::new();
        for (index, points) in frame.samples {
            let series = &doc.overview.series[index];
            tracks
                .entry(series.unit.clone())
                .or_default()
                .push(SignalPlot {
                    signal: series.signal.clone(),
                    label: signal_label(&series.signal).into(),
                    sender: series.sender.clone(),
                    segment: series.segment_id,
                    points,
                });
        }
        let cutoff = doc.overview.session.committed_sequence;
        let reader = RecordingReader::open(path).map_err(|e| e.to_string())?;
        if reader.session().map_err(|e| e.to_string())?.id != doc.overview.session.id {
            return Err("The working recording was replaced".into());
        }
        let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        let (first, total): (i64, i64) = c.query_row(
            "SELECT COALESCE(MIN(id),1),COUNT(*) FROM (SELECT id FROM dtc_scan_result WHERE acquisition_id<=?1 ORDER BY id DESC LIMIT 201)",
            params![cutoff], |r| Ok((r.get(0)?,r.get(1)?))
        ).map_err(|e| e.to_string())?;
        let ecus = c
            .prepare("SELECT id,sender FROM ecu")
            .map_err(|e| e.to_string())?
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|e| e.to_string())?;
        let mut dtcs = Vec::new();
        for scan in reader
            .dtc_scans(&Query {
                after_id: first.saturating_sub(1),
                limit: 201,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|s| s.acquisition_id <= cutoff)
        {
            let sender = scan
                .ecu_id
                .and_then(|id| ecus.get(&id))
                .cloned()
                .unwrap_or_else(|| "unknown".into());
            let codes = reader
                .dtc_entries(
                    scan.id,
                    &Query {
                        limit: 1000,
                        ..Default::default()
                    },
                )
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|e| e.code)
                .collect();
            dtcs.push(DtcRead {
                scan,
                sender,
                codes,
            });
        }
        dtcs.reverse();
        dtcs.truncate(200);
        Ok(Self {
            full,
            view,
            origin_us: doc.overview.origin_us,
            tracks: tracks
                .into_iter()
                .map(|(unit, signals)| UnitTrack { unit, signals })
                .collect(),
            dtcs,
            dtc_history_limited: total > 200,
            raw_series: doc.overview.series.iter().filter(|s| !s.numeric).count(),
            cutoff,
        })
    }
}

/// Keep a requested interval inside the recording, retaining its span when
/// panning against either edge. A wider request fits the complete recording.
pub fn bounded_view(view: Viewport, full: Viewport) -> Viewport {
    let span = (view.to as i128 - view.from as i128)
        .max(1)
        .min(full.to as i128 - full.from as i128);
    let from = (view.from as i128).clamp(full.from as i128, full.to as i128 - span);
    Viewport::new(from as i64, (from + span) as i64)
}
