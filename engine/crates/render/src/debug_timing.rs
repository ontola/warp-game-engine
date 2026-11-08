//! Bounded, data-only timing model for interactive debug interfaces.
//!
//! Feed the existing nonblocking GPU profiler drain once per application frame.
//! GPU readings retain their submitted frame IDs; they are not the current CPU
//! frame. Timestamp interval union, elapsed span, CPU body wall time and frame
//! cadence and main-thread CPU execution remain separate. None of these values
//! is a whole-frame FPS estimate.

use crate::gpu_profile::{GpuPassInterval, GpuProfileDrain, GpuProfileSample, interval_union_ms};
use std::collections::{BTreeMap, BTreeSet};

const MAX_WINDOW: usize = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuState { Disabled, Unsupported, Waiting, Ready, Stale }

#[derive(Clone, Copy, Debug)]
pub struct CpuFrameTiming {
    pub frame_id: u64,
    /// User + kernel CPU execution on the application's frame thread only.
    /// None means unsupported/failed, not zero. Other worker threads excluded.
    pub execution_ms: Option<f64>,
    /// Elapsed wall time of the scoped frame body. This is not CPU execution
    /// cycles: any blocking work within that scope remains part of the value.
    pub frame_body_ms: Option<f64>,
    /// Unclamped frame-entry to frame-entry wall time. Never add it to body/GPU.
    pub cadence_ms: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct PassCost {
    pub name: &'static str,
    pub group: &'static str,
    /// Mean across the GPU window; absent passes count as zero, not a sample
    /// held forever. Same-named overlapping intervals are unioned, not added.
    pub mean_ms: f64,
    pub last_ms: f64,
    pub active_samples: usize,
    /// Explicit parent scopes contribute only time outside their children.
    pub has_exclusive_scope: bool,
}

#[derive(Clone, Debug)]
pub struct TimingSnapshot {
    pub window: usize,
    pub gpu_state: GpuState,
    /// Mean measured first-to-last interval, including gaps.
    pub gpu_interval_ms: Option<f64>,
    /// Mean union of measured intervals; excludes gaps, overlaps counted once.
    pub gpu_union_ms: Option<f64>,
    pub gpu_last_interval_ms: Option<f64>,
    pub gpu_last_union_ms: Option<f64>,
    pub gpu_frame_id: Option<u64>,
    pub gpu_render_sequence: Option<u64>,
    pub gpu_age_frames: Option<u64>,
    pub gpu_samples: usize,
    /// Descending mean cost. Rows can still overlap across independent scopes;
    /// summing this table is never the total measured GPU time.
    pub gpu_passes: Vec<PassCost>,
    pub cpu_frame_ms: Option<f64>,
    pub cpu_last_frame_ms: Option<f64>,
    pub cpu_execution_ms: Option<f64>,
    pub cpu_last_execution_ms: Option<f64>,
    pub cpu_execution_samples: usize,
    pub cadence_ms: Option<f64>,
    pub cpu_samples: usize,
    pub pending: u32,
    pub dropped: u64,
    pub errors: u64,
    pub invalid_gpu_samples: u64,
    pub invalid_cpu_samples: u64,
    pub duplicate_or_old_gpu_samples: u64,
}

struct Frame {
    frame_id: u64,
    sequence: u64,
    output_size: [u32; 2],
    views: u32,
    interval_ms: f64,
    union_ms: f64,
    passes: BTreeMap<&'static str, (f64, bool)>,
}

pub struct TimingModel {
    window: usize,
    stale_after_frames: u64,
    gpu: BTreeMap<u64, Frame>,
    cpu: BTreeMap<u64, CpuFrameTiming>,
    enabled: bool,
    supported: bool,
    pending: u32,
    dropped: u64,
    errors: u64,
    invalid_gpu_samples: u64,
    invalid_cpu_samples: u64,
    duplicate_or_old_gpu_samples: u64,
}

impl Default for TimingModel { fn default() -> Self { Self::new(32) } }

impl TimingModel {
    /// Clamp storage to 1..=240 completed samples. Stale means more than 120
    /// rendered frames behind; the actual age remains visible at all times.
    pub fn new(window: usize) -> Self {
        Self { window: window.clamp(1, MAX_WINDOW), stale_after_frames: 120,
            gpu: BTreeMap::new(), cpu: BTreeMap::new(), enabled: false, supported: false,
            pending: 0, dropped: 0, errors: 0, invalid_gpu_samples: 0,
            invalid_cpu_samples: 0, duplicate_or_old_gpu_samples: 0 }
    }

    /// Call when the renderer/sequence origin changes, or when intentionally
    /// starting a new measurement. CPU scope/cadence history remains separate.
    pub fn clear_gpu(&mut self) {
        self.gpu.clear(); self.pending = 0;
    }

    pub fn record_cpu(&mut self, sample: CpuFrameTiming) {
        if sample.frame_id == u64::MAX || [sample.execution_ms, sample.frame_body_ms, sample.cadence_ms]
            .into_iter().flatten().any(|value| !valid_ms(value))
        {
            self.invalid_cpu_samples += 1; return;
        }
        if self.cpu.contains_key(&sample.frame_id) { return; }
        self.cpu.insert(sample.frame_id, sample);
        while self.cpu.len() > self.window { self.cpu.pop_first(); }
    }

    /// No device work or blocking readback. The caller can pass the same drain
    /// to its logger; this model only borrows and validates completed samples.
    pub fn ingest_gpu(&mut self, drain: &GpuProfileDrain, current_frame_id: u64) {
        self.enabled = drain.enabled; self.supported = drain.supported;
        self.pending = drain.pending; self.dropped = drain.dropped; self.errors = drain.errors;
        // Drain order need not be completion/submission order. Latest identity
        // wins, while still-relevant late samples may fill the averaging window.
        let mut samples: Vec<_> = drain.samples.iter().collect();
        samples.sort_unstable_by_key(|sample| sample.render_sequence);
        for sample in samples {
            if self.gpu.contains_key(&sample.render_sequence) || (self.gpu.len() >= self.window
                && self.gpu.first_key_value().is_some_and(|(&sequence, _)| sample.render_sequence <= sequence))
            {
                self.duplicate_or_old_gpu_samples += 1; continue;
            }
            let Some(frame) = decode_frame(sample, current_frame_id) else {
                self.invalid_gpu_samples += 1; continue;
            };
            if let Some((_, latest)) = self.gpu.last_key_value() {
                if frame.sequence > latest.sequence && frame.frame_id < latest.frame_id {
                    self.invalid_gpu_samples += 1; continue;
                }
                if frame.output_size != latest.output_size || frame.views != latest.views {
                    if frame.sequence < latest.sequence {
                        self.duplicate_or_old_gpu_samples += 1; continue;
                    }
                    // Do not average a new split/resolution layout with the old.
                    self.gpu.clear();
                }
            }
            self.gpu.insert(frame.sequence, frame);
            while self.gpu.len() > self.window { self.gpu.pop_first(); }
        }
    }

    pub fn snapshot(&self, current_frame_id: u64) -> TimingSnapshot {
        let latest = self.gpu.last_key_value().map(|(_, frame)| frame);
        let age = latest.map(|frame| current_frame_id.saturating_sub(frame.frame_id));
        let state = if !self.supported { GpuState::Unsupported }
            else if !self.enabled { GpuState::Disabled }
            else if latest.is_none() { GpuState::Waiting }
            else if age.is_some_and(|age| age > self.stale_after_frames) { GpuState::Stale }
            else { GpuState::Ready };
        let names: BTreeSet<_> = self.gpu.values().flat_map(|frame| frame.passes.keys().copied()).collect();
        let mut passes: Vec<_> = names.into_iter().map(|name| {
            let mut sum = 0.0; let mut active = 0; let mut exclusive = false;
            for frame in self.gpu.values() {
                if let Some(&(cost, scope)) = frame.passes.get(name) {
                    sum += cost; active += 1; exclusive |= scope;
                }
            }
            PassCost { name, group: pass_group(name), mean_ms: sum / self.gpu.len() as f64,
                last_ms: latest.and_then(|frame| frame.passes.get(name)).map_or(0.0, |value| value.0),
                active_samples: active, has_exclusive_scope: exclusive }
        }).collect();
        passes.sort_unstable_by(|a, b| b.mean_ms.total_cmp(&a.mean_ms).then_with(|| a.name.cmp(b.name)));
        TimingSnapshot {
            window: self.window, gpu_state: state,
            gpu_interval_ms: mean(self.gpu.values().map(|frame| frame.interval_ms)),
            gpu_union_ms: mean(self.gpu.values().map(|frame| frame.union_ms)),
            gpu_last_interval_ms: latest.map(|frame| frame.interval_ms),
            gpu_last_union_ms: latest.map(|frame| frame.union_ms),
            gpu_frame_id: latest.map(|frame| frame.frame_id), gpu_render_sequence: latest.map(|frame| frame.sequence),
            gpu_age_frames: age, gpu_samples: self.gpu.len(), gpu_passes: passes,
            cpu_frame_ms: mean(self.cpu.values().filter_map(|sample| sample.frame_body_ms)),
            cpu_last_frame_ms: self.cpu.last_key_value().and_then(|(_, sample)| sample.frame_body_ms),
            cpu_execution_ms: mean(self.cpu.values().filter_map(|sample| sample.execution_ms)),
            cpu_last_execution_ms: self.cpu.last_key_value().and_then(|(_, sample)| sample.execution_ms),
            cpu_execution_samples: self.cpu.values().filter(|sample|sample.execution_ms.is_some()).count(),
            cadence_ms: mean(self.cpu.values().filter_map(|sample| sample.cadence_ms)), cpu_samples: self.cpu.len(),
            pending: self.pending, dropped: self.dropped, errors: self.errors,
            invalid_gpu_samples: self.invalid_gpu_samples, invalid_cpu_samples: self.invalid_cpu_samples,
            duplicate_or_old_gpu_samples: self.duplicate_or_old_gpu_samples,
        }
    }
}

fn valid_ms(value: f64) -> bool { value.is_finite() && value >= 0.0 }
fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, count) = values.fold((0.0, 0usize), |(sum, count), value| (sum + value, count + 1));
    (count > 0).then(|| sum / count as f64).filter(|value| value.is_finite())
}

fn decode_frame(sample: &GpuProfileSample, current_frame_id: u64) -> Option<Frame> {
    if sample.frame_id == u64::MAX || sample.frame_id > current_frame_id || sample.intervals.is_empty()
        || sample.output_size.contains(&0) || sample.views == 0
        || !valid_ms(sample.instrumented_gpu_ms) || !valid_ms(sample.covered_interval_ms)
        || !valid_ms(sample.covered_union_ms)
        || sample.gpu_ms.iter().any(|(name, value)| name.is_empty() || !valid_ms(*value))
    { return None; }
    let union = interval_union_ms(sample.intervals.iter().map(|interval| (interval.start_ms, interval.end_ms)))?;
    let lo = sample.intervals.iter().map(|interval| interval.start_ms).fold(f64::INFINITY, f64::min);
    let hi = sample.intervals.iter().map(|interval| interval.end_ms).fold(0.0, f64::max);
    let span = hi - lo;
    let sum: f64 = sample.intervals.iter().filter(|interval| interval.parent.is_none())
        .map(|interval| interval.end_ms - interval.start_ms).sum();
    // A small relative rounding allowance; no invalid summary becomes zero.
    let equal = |a: f64, b: f64| (a - b).abs() <= 1e-8 * a.abs().max(b.abs()).max(1.0);
    if !sum.is_finite() || !equal(span, sample.covered_interval_ms)
        || !equal(union, sample.covered_union_ms) || !equal(sum, sample.instrumented_gpu_ms)
    { return None; }
    let mut ranges: BTreeMap<&'static str, (Vec<(f64, f64)>, bool)> = BTreeMap::new();
    for (index, interval) in sample.intervals.iter().enumerate() {
        if interval.name.is_empty() || interval.view_index.is_some_and(|view| view >= sample.views) { return None; }
        if let Some(parent) = interval.parent {
            // Authored parent-before-child indices make cycles impossible.
            if parent >= index { return None; }
            let scope = &sample.intervals[parent];
            if scope.start_ms > interval.start_ms || scope.end_ms < interval.end_ms
                || scope.view_index != interval.view_index { return None; }
        }
        let children: Vec<_> = sample.intervals.iter().filter(|child| child.parent == Some(index))
            .map(|child| (child.start_ms, child.end_ms)).collect();
        let entry = ranges.entry(interval.name).or_default();
        entry.1 |= !children.is_empty();
        entry.0.extend(exclusive_ranges(interval, children));
    }
    let mut passes = BTreeMap::new();
    for (name, (ranges, exclusive)) in ranges {
        passes.insert(name, (interval_union_ms(ranges)?, exclusive));
    }
    Some(Frame { frame_id: sample.frame_id, sequence: sample.render_sequence,
        output_size: sample.output_size, views: sample.views, interval_ms: span, union_ms: union, passes })
}

fn exclusive_ranges(interval: &GpuPassInterval, mut children: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    children.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let mut cursor = interval.start_ms;
    let mut output = Vec::new();
    for (start, end) in children {
        if start > cursor { output.push((cursor, start)); }
        cursor = cursor.max(end);
    }
    if cursor < interval.end_ms { output.push((cursor, interval.end_ms)); }
    output
}

/// The feature a GPU pass belongs to, for grouped cost views (the F10 pass
/// list, the F3 stats overlay). Unknown names land in "Other".
pub fn pass_group(name: &str) -> &'static str {
    if name.contains("shadow") || name.starts_with("foliage page") || name.contains(" sun ") || name.starts_with("far sun")
        || name.starts_with("static sun") || name.starts_with("sun ") { "Shadows" }
    else if name.starts_with("shade") || name.contains("radiance") || name.contains("volumetric") { "Lighting" }
    else if name.contains("terrain") && !name.contains("foliage") { "Terrain" }
    else if name.contains("grass") || name.contains("meadow") || name.contains("voxel")
        || name.contains("wind") || name.contains("vegetation") || name.contains("canopy")
        || name.contains("foliage") || name.contains("forest") || name.contains("tree") || name.contains("branch") { "Vegetation" }
    else if name.contains("cull") || name.contains("raster") || name.contains("broadphase")
        || name.contains("occlusion pyramid") || name.contains("visibility") || name.contains("refit") { "Geometry" }
    else if matches!(name, "taa" | "dlss" | "tonemap" | "auto exposure" | "motion vectors" | "motion blur" | "depth of field" | "bloom") { "Post-processing" }
    else if name.contains("particles") || name.contains("exhaust") || name.contains("transparency")
        || name.contains("animation") || name.contains("skinning") || name.contains("crowd") || name.contains("debris")
        || matches!(name, "fx" | "sprites") { "Effects" }
    else { "Other" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(name: &'static str, start_ms: f64, end_ms: f64, parent: Option<usize>) -> GpuPassInterval {
        GpuPassInterval { name, start_ms, end_ms, parent, view_index: None }
    }
    fn sample(frame_id: u64, sequence: u64, intervals: Vec<GpuPassInterval>) -> GpuProfileSample {
        let lo = intervals.iter().map(|interval| interval.start_ms).fold(f64::INFINITY, f64::min);
        let hi = intervals.iter().map(|interval| interval.end_ms).fold(0.0, f64::max);
        GpuProfileSample { frame_id, render_sequence: sequence,
            gpu_ms: intervals.iter().filter(|i| i.parent.is_none()).map(|i| (i.name, i.end_ms - i.start_ms)).collect(),
            instrumented_gpu_ms: intervals.iter().filter(|i| i.parent.is_none()).map(|i| i.end_ms - i.start_ms).sum(),
            covered_interval_ms: hi - lo,
            covered_union_ms: interval_union_ms(intervals.iter().map(|i| (i.start_ms, i.end_ms))).unwrap(),
            intervals, output_size: [1920, 1080], views: 1 }
    }
    fn drain(samples: Vec<GpuProfileSample>) -> GpuProfileDrain {
        GpuProfileDrain { samples, enabled: true, supported: true, ..Default::default() }
    }
    fn simple(frame: u64, ms: f64) -> GpuProfileSample {
        sample(frame, frame, vec![interval("shade", 0.0, ms, None)])
    }

    #[test]
    fn nested_and_independent_overlaps_never_become_an_additive_gpu_total() {
        let mut model = TimingModel::default();
        let measured = sample(10, 7, vec![interval("render", 0.0, 10.0, None),
            interval("meadow render", 1.0, 5.0, Some(0)), interval("shade", 4.0, 8.0, Some(0)),
            interval("tonemap", 12.0, 14.0, None)]);
        assert_eq!(measured.instrumented_gpu_ms, 12.0, "authored children do not inflate root totals");
        model.ingest_gpu(&drain(vec![measured]), 12);
        let snapshot = model.snapshot(12);
        assert_eq!(snapshot.gpu_interval_ms, Some(14.0));
        assert_eq!(snapshot.gpu_union_ms, Some(12.0));
        assert_eq!(snapshot.gpu_age_frames, Some(2));
        let parent = snapshot.gpu_passes.iter().find(|pass| pass.name == "render").unwrap();
        assert_eq!(parent.mean_ms, 3.0); assert!(parent.has_exclusive_scope);
        assert_eq!(snapshot.gpu_passes.iter().map(|pass| pass.mean_ms).sum::<f64>(), 13.0,
            "independent overlapping child costs still must not be summed as total");
    }

    #[test]
    fn explicit_children_cannot_cross_submitted_view_boundaries() {
        let mut parent = interval("aggregate", 0.0, 10.0, None);
        parent.view_index = Some(0);
        let mut child = interval("aggregate depth", 0.0, 3.0, Some(0));
        child.view_index = Some(0);
        let mut measured = sample(10, 7, vec![parent, child]);
        assert!(decode_frame(&measured, 10).is_some());
        measured.views = 4;
        measured.intervals[1].view_index = Some(1);
        assert!(decode_frame(&measured, 10).is_none());
        measured.intervals[1].view_index = Some(0);
        measured.intervals[0].view_index = Some(4);
        assert!(decode_frame(&measured, 10).is_none());
    }

    #[test]
    fn asynchronous_order_ids_window_and_missing_passes_remain_explicit() {
        let mut model = TimingModel::new(3);
        model.ingest_gpu(&drain(vec![simple(12, 12.0), simple(10, 10.0)]), 13);
        model.ingest_gpu(&drain(vec![simple(11, 11.0)]), 14);
        let snapshot = model.snapshot(14);
        assert_eq!(snapshot.gpu_frame_id, Some(12)); assert_eq!(snapshot.gpu_age_frames, Some(2));
        assert_eq!(snapshot.gpu_union_ms, Some(11.0)); assert_eq!(snapshot.gpu_samples, 3);
        model.ingest_gpu(&drain(vec![sample(13, 13, vec![interval("tonemap", 0.0, 2.0, None)])]), 14);
        model.ingest_gpu(&drain(vec![simple(12, 99.0), simple(9, 99.0)]), 14);
        let snapshot = model.snapshot(14);
        let shade = snapshot.gpu_passes.iter().find(|pass| pass.name == "shade").unwrap();
        assert_eq!(shade.mean_ms, 23.0 / 3.0); assert_eq!(shade.last_ms, 0.0); assert_eq!(shade.active_samples, 2);
        assert_eq!(snapshot.duplicate_or_old_gpu_samples, 2);
        assert_eq!(model.snapshot(134).gpu_state, GpuState::Stale);
    }

    #[test]
    fn invalid_and_missing_samples_never_turn_into_zero_gpu_time() {
        let mut model = TimingModel::default();
        assert_eq!(model.snapshot(0).gpu_state, GpuState::Unsupported);
        model.ingest_gpu(&drain(Vec::new()), 1);
        assert_eq!(model.snapshot(1).gpu_state, GpuState::Waiting);
        assert_eq!(model.snapshot(1).gpu_union_ms, None);
        for kind in 0..6 {
            let mut bad = simple(kind, 2.0);
            match kind {
                0 => bad.intervals[0].end_ms = f64::NAN,
                1 => bad.intervals[0].start_ms = -1.0,
                2 => bad.covered_union_ms = 0.0,
                3 => bad.intervals[0].parent = Some(0),
                4 => bad.frame_id = 99,
                _ => bad.intervals.clear(),
            }
            model.ingest_gpu(&drain(vec![bad]), 10);
        }
        assert_eq!(model.snapshot(10).invalid_gpu_samples, 6);
        assert_eq!(model.snapshot(10).gpu_union_ms, None);
        model.ingest_gpu(&drain(vec![simple(11, 0.0)]), 11);
        assert_eq!(model.snapshot(11).gpu_union_ms, Some(0.0), "a measured zero is different from missing data");
        model.ingest_gpu(&GpuProfileDrain { enabled: false, supported: true, ..Default::default() }, 12);
        assert_eq!(model.snapshot(12).gpu_state, GpuState::Disabled);
    }

    #[test]
    fn cpu_body_cadence_and_gpu_history_are_never_added_or_joined_by_arrival() {
        let mut model = TimingModel::new(2);
        model.record_cpu(CpuFrameTiming { frame_id: 1, execution_ms:Some(1.0), frame_body_ms: Some(3.0), cadence_ms: None });
        model.record_cpu(CpuFrameTiming { frame_id: 2, execution_ms:Some(2.0), frame_body_ms: Some(5.0), cadence_ms: Some(16.0) });
        model.record_cpu(CpuFrameTiming { frame_id: 3, execution_ms:None, frame_body_ms: Some(f64::INFINITY), cadence_ms: Some(20.0) });
        model.ingest_gpu(&drain(vec![simple(1, 8.0)]), 2);
        let snapshot = model.snapshot(2);
        assert_eq!(snapshot.cpu_frame_ms, Some(4.0)); assert_eq!(snapshot.cpu_last_frame_ms, Some(5.0));
        assert_eq!(snapshot.cpu_execution_ms,Some(1.5));assert_eq!(snapshot.cpu_last_execution_ms,Some(2.0));
        assert_eq!(snapshot.cadence_ms, Some(16.0)); assert_eq!(snapshot.gpu_union_ms, Some(8.0));
        assert_eq!(snapshot.invalid_cpu_samples, 1); assert_eq!(snapshot.gpu_age_frames, Some(1));
        model.clear_gpu();
        assert_eq!(model.snapshot(2).cpu_frame_ms, Some(4.0));
        assert_eq!(model.snapshot(2).gpu_union_ms, None);
    }

    #[test]
    fn cpu_execution_averages_only_measured_samples_and_rejects_invalid_values() {
        let mut model=TimingModel::new(3);
        model.record_cpu(CpuFrameTiming {frame_id:1,execution_ms:None,frame_body_ms:Some(10.0),cadence_ms:None});
        assert_eq!(model.snapshot(1).cpu_execution_ms,None);
        assert_eq!(model.snapshot(1).cpu_execution_samples,0);
        model.record_cpu(CpuFrameTiming {frame_id:2,execution_ms:Some(2.0),frame_body_ms:Some(20.0),cadence_ms:None});
        model.record_cpu(CpuFrameTiming {frame_id:3,execution_ms:Some(0.0),frame_body_ms:Some(30.0),cadence_ms:None});
        let snapshot=model.snapshot(3);
        assert_eq!(snapshot.cpu_execution_ms,Some(1.0));assert_eq!(snapshot.cpu_execution_samples,2);
        assert_eq!(snapshot.cpu_frame_ms,Some(20.0));
        for (i,value) in [f64::NAN,f64::INFINITY,-0.1].into_iter().enumerate() {
            model.record_cpu(CpuFrameTiming {frame_id:4+i as u64,execution_ms:Some(value),frame_body_ms:Some(10.0),cadence_ms:None});
        }
        assert_eq!(model.snapshot(6).invalid_cpu_samples,3);
        model.record_cpu(CpuFrameTiming {frame_id:7,execution_ms:None,frame_body_ms:Some(40.0),cadence_ms:None});
        let snapshot=model.snapshot(7);
        assert_eq!(snapshot.cpu_execution_ms,Some(1.0));assert_eq!(snapshot.cpu_last_execution_ms,None);
        assert_eq!(snapshot.cpu_frame_ms,Some(30.0));assert_eq!(snapshot.cpu_samples,3);
    }

    #[test]
    fn layout_change_starts_fresh_and_same_named_overlap_is_counted_once() {
        let mut model = TimingModel::default();
        model.ingest_gpu(&drain(vec![simple(1, 100.0)]), 2);
        let mut split = sample(2, 2, vec![interval("shade", 0.0, 5.0, None), interval("shade", 3.0, 8.0, None)]);
        split.views = 4;
        model.ingest_gpu(&drain(vec![split]), 3);
        let snapshot = model.snapshot(3);
        assert_eq!(snapshot.gpu_samples, 1); assert_eq!(snapshot.gpu_union_ms, Some(8.0));
        assert_eq!(snapshot.gpu_passes[0].mean_ms, 8.0);
        model.ingest_gpu(&drain(vec![simple(1, 100.0)]), 3);
        assert_eq!(model.snapshot(3).gpu_samples, 1);
    }
}
