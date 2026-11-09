use crate::model::Viewport;
use crate::pid_view::{DerivedSeries, PidReader};
use kittx::datastore::*;
use std::{collections::BTreeSet, path::PathBuf};
#[derive(Clone)]
pub struct Document {
    pub path: PathBuf,
    pub overview: RecordingOverview,
    derived: Vec<Option<DerivedSeries>>,
}
pub struct Frame {
    pub samples: Vec<(usize, Vec<PlotSample>)>,
    pub scans: Vec<DtcMarker>,
    pub view: Viewport,
}
impl Document {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let reader = RecordingReader::open(&path).map_err(|e| e.to_string())?;
        let mut overview = reader.overview().map_err(|e| e.to_string())?;
        let derived =
            PidReader::open(&path, &overview.session.id)?.extend_overview(&mut overview)?;
        Ok(Self {
            path,
            overview,
            derived,
        })
    }
    fn reader(&self) -> Result<RecordingReader, String> {
        let reader = RecordingReader::open(&self.path).map_err(|e| e.to_string())?;
        if reader.session().map_err(|e| e.to_string())?.id != self.overview.session.id {
            return Err("The recording file was replaced; reopen it".into());
        }
        Ok(reader)
    }
    pub fn full(&self) -> Viewport {
        let (first, last) = (self.overview.first_us, self.overview.last_us);
        if first == last {
            Viewport::new(first.saturating_sub(500_000), last.saturating_add(500_000))
        } else {
            Viewport::new(first, last)
        }
    }
    pub fn frame(
        &self,
        view: Viewport,
        selected: &[usize],
        pixels: u32,
        mode: DtcDisplayMode,
    ) -> Result<Frame, String> {
        self.frame_with_cancel(view, selected, pixels, mode, || false)
    }
    /// Cancel between bounded series queries. A SQLite query already executing completes first.
    pub fn frame_with_cancel(
        &self,
        view: Viewport,
        selected: &[usize],
        pixels: u32,
        mode: DtcDisplayMode,
        cancelled: impl Fn() -> bool,
    ) -> Result<Frame, String> {
        let r = self.reader()?;
        let cutoff = self.overview.session.committed_sequence;
        if selected.len() > 4096 || selected.iter().collect::<BTreeSet<_>>().len() != selected.len()
        {
            return Err("Select at most 4096 distinct series per viewport".into());
        }
        let buckets =
            (24_000 / selected.len().max(1)).clamp(1, pixels.clamp(1, 4096) as usize) as u32;
        let mut samples = Vec::new();
        let decoded = PidReader::open(&self.path, &self.overview.session.id)?;
        for &i in selected {
            if cancelled() {
                return Err("Superseded viewport".into());
            }
            let series = self.overview.series.get(i).ok_or("Unknown series")?;
            samples.push((
                i,
                if let Some(source) = &self.derived[i] {
                    decoded.plot(source, cutoff, view.from, view.to, buckets)?
                } else {
                    r.plot_samples(series, cutoff, view.from, view.to, buckets)
                        .map_err(|e| e.to_string())?
                },
            ));
        }
        if cancelled() {
            return Err("Superseded viewport".into());
        }
        let scans = r
            .dtc_markers_mode(cutoff, view.from, view.to, pixels.clamp(1, 4096), mode)
            .map_err(|e| e.to_string())?;
        Ok(Frame {
            samples,
            scans,
            view,
        })
    }
    pub fn inspect(&self, at: i64, selected: &[usize]) -> Result<String, String> {
        let r = self.reader()?;
        let decoded = PidReader::open(&self.path, &self.overview.session.id)?;
        let mut out = format!(
            "Elapsed {:.6} s · monotonic origin {} µs\n",
            (at - self.overview.origin_us) as f64 / 1e6,
            at
        );
        for &i in selected {
            let s = self.overview.series.get(i).ok_or("Unknown series")?;
            let point = if let Some(source) = &self.derived[i] {
                decoded.nearest(source, self.overview.session.committed_sequence, at)?
            } else {
                r.nearest_sample(s, self.overview.session.committed_sequence, at)
                    .map_err(|e| e.to_string())?
            };
            if let Some(p) = point {
                out += &format!(
                    "{} ({}) · ECU {} · {} {} · UTC {} µs · sample {} · window {}–{} µs\n",
                    crate::model::signal_label(&s.signal),
                    s.signal,
                    s.sender,
                    p.value,
                    s.unit,
                    p.utc_us,
                    p.id,
                    p.started_us,
                    p.completed_us
                )
            }
        }
        Ok(out)
    }
    pub fn scan_details(&self, scan: &DtcScanRow) -> Result<String, String> {
        let r = self.reader()?;
        let scan = self.full_scan(&r, scan.id)?;
        self.scan_text(&r, &scan)
    }
    fn full_scan(&self, reader: &RecordingReader, id: i64) -> Result<DtcScanRow, String> {
        reader
            .dtc_scans(&Query {
                after_id: id.saturating_sub(1),
                limit: 1,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|scan| {
                scan.id == id && scan.acquisition_id <= self.overview.session.committed_sequence
            })
            .ok_or_else(|| "DTC scan is not in this recording snapshot".into())
    }
    fn scan_text(&self, r: &RecordingReader, scan: &DtcScanRow) -> Result<String, String> {
        let entries = r
            .dtc_entries(
                scan.id,
                &Query {
                    limit: 1000,
                    ..Query::default()
                },
            )
            .map_err(|e| e.to_string())?;
        let codes = entries
            .iter()
            .map(|entry| entry.code.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let mut out = format!(
            "DTC read\nCodes: {}\nObserved: {:.6} s from recording start\nUTC: {} µs\nCategory: {} · namespace: {}\nECU: {} · result: {} · code count: {}\nScan: {}\n",
            if codes.is_empty() { "-" } else { &codes },
            (i128::from(scan.elapsed_us) - i128::from(self.overview.origin_us)) as f64 / 1e6,
            scan.utc_us,
            scan.category,
            scan.namespace,
            scan.ecu_id.map_or_else(|| "-".into(), |id| id.to_string()),
            scan.outcome,
            scan.code_count
                .map_or_else(|| "-".into(), |count| count.to_string()),
            scan.id,
        );
        if let Some(error) = &scan.error {
            out += &format!("Error: {error}\n");
        }
        if let Some(profile) = &scan.profile_id {
            out += &format!("Profile: {profile}\n");
        }
        if entries.len() == 1000 {
            out += "First 1000 DTC entries shown; source retains all entries.\n";
        }
        for e in entries {
            out += &format!(
                "{} · description: {} · bytes {:02X?}\n",
                e.code,
                e.interpretation.as_deref().unwrap_or("-"),
                e.bytes
            )
        }
        if let Ok(e) = r.evidence(scan.acquisition_id) {
            out += &format!(
                "Evidence partial={} truncated={} · raw response (first 4096 bytes)\n{}\n{}",
                e.partial,
                e.truncated,
                e.raw_response.as_ref().map_or_else(
                    || "-".into(),
                    |raw| format!("{:02X?}", &raw[..raw.len().min(4096)])
                ),
                e.context
            )
        }
        if let Some(raw) = &scan.raw_payload {
            out += &format!(
                "\nRaw payload (display limited to 4096 bytes) {:02X?}",
                &raw[..raw.len().min(4096)]
            )
        }
        Ok(out)
    }
}
impl Document {
    pub fn cluster_details(&self, marker: &DtcMarker) -> Result<String, String> {
        self.marker_details(std::slice::from_ref(marker))
    }
    /// Inspect the exact groups represented by a painted point, retaining ECU and
    /// category identity. At most 100 unique scans are expanded per selection.
    pub fn marker_details(&self, markers: &[DtcMarker]) -> Result<String, String> {
        let r = self.reader()?;
        let mut out = String::new();
        let mut shown = BTreeSet::new();
        for marker in markers {
            if shown.len() >= 100 {
                out += "More observations remain; zoom in to inspect a smaller interval.\n";
                break;
            }
            // A single projected marker must not select a different read in its bucket.
            if marker.scans <= 1 {
                let scan = self.full_scan(&r, marker.scan.id)?;
                if shown.insert(scan.id) {
                    out += &self.scan_text(&r, &scan)?;
                    out.push('\n');
                }
                continue;
            }
            out += &format!(
                "Read history: {} scans · observed {:.6}–{:.6} s · {} / {}\n",
                marker.scans,
                (i128::from(marker.scan.elapsed_us) - i128::from(self.overview.origin_us)) as f64
                    / 1e6,
                (i128::from(marker.last_us) - i128::from(self.overview.origin_us)) as f64 / 1e6,
                marker.scan.category,
                marker.scan.namespace,
            );
            let mut q = Query {
                limit: 100,
                from_elapsed_us: Some(marker.scan.elapsed_us.max(0)),
                to_elapsed_us: Some(marker.last_us.max(marker.scan.elapsed_us)),
                ecu_id: marker.scan.ecu_id,
                ..Query::default()
            };
            loop {
                let page = r.dtc_scans(&q).map_err(|e| e.to_string())?;
                if page.is_empty() {
                    break;
                }
                q.after_id = page.last().unwrap().id;
                for scan in page {
                    if scan.acquisition_id > self.overview.session.committed_sequence
                        || scan.ecu_id != marker.scan.ecu_id
                        || scan.category != marker.scan.category
                        || scan.namespace != marker.scan.namespace
                        || scan.outcome != marker.scan.outcome
                        || scan.code_count.map(|c| c > 0) != marker.scan.code_count.map(|c| c > 0)
                        || !shown.insert(scan.id)
                    {
                        continue;
                    }
                    out += &self.scan_text(&r, &scan)?;
                    out.push('\n');
                    if shown.len() >= 100 {
                        out += "More observations may remain; zoom in to inspect a smaller interval.\n";
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
}
impl Document {
    /// Display a bounded prefix of a raw series sample without interpreting vendor bytes.
    pub fn raw_sample(&self, index: usize) -> Result<String, String> {
        let series = self.overview.series.get(index).ok_or("Unknown series")?;
        let series = self.derived[index]
            .as_ref()
            .map_or(series, |derived| &derived.source);
        let r = self.reader()?;
        let q = Query {
            limit: 1,
            ecu_id: Some(series.ecu_id),
            signal_id: Some(series.signal.clone()),
            ..Query::default()
        };
        let rows = r.measurements(&q).map_err(|e| e.to_string())?;
        let Some(row) = rows
            .first()
            .filter(|m| m.acquisition_id <= self.overview.session.committed_sequence)
        else {
            return Ok("No raw samples before cutoff".into());
        };
        let bytes = row.bytes.as_deref().unwrap_or_default();
        Ok(format!(
            "{} · ECU {} · sample {} · UTC {} µs · elapsed {} µs\nRaw bytes ({} total; first 4096 shown): {:02X?}",
            series.signal,
            series.sender,
            row.id,
            row.utc_us,
            row.elapsed_us,
            bytes.len(),
            &bytes[..bytes.len().min(4096)]
        ))
    }
}
