use kittx::datastore::{DtcMarker, PlotSample};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub from: i64,
    pub to: i64,
}
impl Viewport {
    pub fn new(from: i64, to: i64) -> Self {
        let from = from.min(i64::MAX - 1);
        Self {
            from,
            to: to.max(from + 1),
        }
    }
    pub fn time(self, fraction: f64) -> i64 {
        self.from.saturating_add(
            (((self.to as i128 - self.from as i128) as f64) * fraction.clamp(0., 1.)) as i64,
        )
    }
    pub fn fraction(self, time: i64) -> f32 {
        ((time as i128 - self.from as i128) as f64 / (self.to as i128 - self.from as i128) as f64)
            as f32
    }
    pub fn zoom(self, factor: f64, anchor: f64) -> Self {
        let center = self.time(anchor);
        let span = ((self.to as i128 - self.from as i128) as f64 * factor)
            .clamp(10., i64::MAX as f64 / 4.) as i64;
        let from = center.saturating_sub((span as f64 * anchor) as i64);
        Self::new(from, from.saturating_add(span))
    }
    pub fn pan(self, fraction: f64) -> Self {
        let delta = ((self.to as i128 - self.from as i128) as f64 * fraction) as i64;
        Self::new(
            self.from.saturating_add(delta),
            self.to.saturating_add(delta),
        )
    }
}
pub fn range(samples: impl Iterator<Item = f64>) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in samples.filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() {
        return (0., 1.);
    }
    let pad = ((hi - lo) * 0.05).max(lo.abs() * 0.01).max(0.01);
    (lo - pad, hi + pad)
}
pub fn gap(a: &PlotSample, b: &PlotSample, _period: Option<i64>) -> bool {
    a.partial || b.partial || a.run_id != b.run_id
}

/// Known standard signal names; unknown decoder IDs are retained verbatim.
pub fn signal_label(id: &str) -> &str {
    if let Some(field) = crate::pid::field(id) {
        return field.name;
    }
    match id {
        "01:04" => "Engine load",
        "01:05" => "Coolant temperature",
        "01:0B" => "Manifold pressure",
        "01:0C" => "Engine RPM",
        "01:0D" => "Vehicle speed",
        "01:0F" => "Intake temperature",
        "01:11" => "Throttle position",
        "adapter:voltage" => "Adapter battery voltage",
        _ => id,
    }
}

/// A diagnostic read's visual meaning, without guessing fault onset or vendor codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MarkerKind {
    Positive,
    Empty,
    Failed,
    Raw,
}
pub fn marker_kind(scan: &kittx::datastore::DtcScanRow) -> MarkerKind {
    if scan.outcome == "raw_only" {
        MarkerKind::Raw
    } else if scan.outcome != "ok" {
        MarkerKind::Failed
    } else if scan.code_count.is_some_and(|n| n > 0) {
        MarkerKind::Positive
    } else {
        MarkerKind::Empty
    }
}

/// A visible DTC point. Nearby reads of the same kind share one hit target;
/// members retain their original ECU, category and observation interval.
#[derive(Debug, Clone)]
pub struct PlotDtcMarker {
    pub id: i64,
    pub x: f32,
    pub kind: MarkerKind,
    pub scans: i64,
    pub members: Vec<DtcMarker>,
}

/// Build the same bounded pixel groups for drawing and mouse hit testing.
pub fn plot_dtc_markers<'a>(
    scans: impl IntoIterator<Item = &'a DtcMarker>,
    view: Viewport,
    width: f32,
) -> Vec<PlotDtcMarker> {
    if !width.is_finite() || width <= 0. {
        return Vec::new();
    }
    let edge = width.min(12.) / 2.;
    let mut groups = BTreeMap::<(i32, MarkerKind), PlotDtcMarker>::new();
    for marker in scans {
        if !(view.from..=view.to).contains(&marker.scan.elapsed_us) {
            continue;
        }
        let x = (view.fraction(marker.scan.elapsed_us) * width).clamp(edge, width - edge);
        let kind = marker_kind(&marker.scan);
        let group = groups
            .entry(((x / 12.).floor() as i32, kind))
            .or_insert_with(|| PlotDtcMarker {
                id: marker.scan.id,
                x,
                kind,
                scans: 0,
                members: Vec::new(),
            });
        group.scans = group.scans.saturating_add(marker.scans);
        group.members.push(marker.clone());
    }
    groups.into_values().collect()
}

/// Hit a point within nine pixels of its painted center in the bottom DTC lane.
/// Equal-distance overlaps select the point painted last.
pub fn hit_dtc_marker(markers: &[PlotDtcMarker], x: f32, y: f32, height: f32) -> Option<usize> {
    if !x.is_finite()
        || !y.is_finite()
        || !height.is_finite()
        || x < 0.
        || height <= 0.
        || (y - (height - 12.)).abs() > 9.
    {
        return None;
    }
    markers
        .iter()
        .enumerate()
        .filter(|(_, marker)| (marker.x - x).abs() <= 9.)
        .min_by(|(a_index, a), (b_index, b)| {
            (a.x - x)
                .abs()
                .total_cmp(&(b.x - x).abs())
                .then_with(|| b_index.cmp(a_index))
        })
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zoom_anchor_pan_precision() {
        let v = Viewport::new(9_000_000_000_000, 9_000_001_000_000);
        assert_eq!(v.time(0.5), 9_000_000_500_000);
        let z = v.zoom(0.5, 0.25);
        assert_eq!(z.time(0.25), v.time(0.25));
        assert_eq!(z.pan(1.).from, z.to);
        assert_eq!(v.fraction(v.to), 1.);
    }
    #[test]
    fn finite_ranges() {
        assert_eq!(range([f64::NAN].into_iter()), (0., 1.));
        let (a, b) = range([-5., -5.].into_iter());
        assert!(a < -5. && b > -5.);
    }
}
