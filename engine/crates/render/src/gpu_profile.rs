//! Delayed GPU timestamps for interactive frames. Four readback slots bound both
//! memory and outstanding work; a busy ring drops samples instead of waiting.
//! The interval covers instrumented rendering, not presentation or whole-frame FPS.

use crate::Gpu;
use std::sync::{Arc, atomic::{AtomicU8, Ordering}};

const RING_SIZE: usize = 4;
const WAITING: u8 = 0;
const READY: u8 = 1;
const FAILED: u8 = 2;

/// Timestamp interval relative to this submitted frame's first measured start.
/// Explicit parent indices are optional; temporal containment alone does not
/// establish a hierarchy because independent GPU passes may overlap.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuPassInterval {
    pub name: &'static str,
    pub start_ms: f64,
    pub end_ms: f64,
    /// Index of the enclosing instrumentation scope, when one was authored.
    pub parent: Option<usize>,
    /// Submitted-frame view index; None denotes shared work (including lamps).
    pub view_index: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct GpuProfileSample {
    pub frame_id: u64,
    pub render_sequence: u64,
    /// Same-named root scopes across all views are summed. Authored children
    /// are detail only, so adding them preserves the legacy pass totals.
    pub gpu_ms: Vec<(&'static str, f64)>,
    /// Sum of root scope intervals; children are not counted a second time.
    /// Root envelopes can include gaps and independent scopes may overlap.
    pub instrumented_gpu_ms: f64,
    /// Earliest measured start to latest measured end, including any gaps.
    pub covered_interval_ms: f64,
    /// Union of measured intervals: excludes gaps and counts overlaps once.
    /// This is timestamp coverage, not hardware utilization or whole-frame FPS.
    pub covered_union_ms: f64,
    pub intervals: Vec<GpuPassInterval>,
    pub output_size: [u32; 2],
    pub views: u32,
}

impl GpuProfileSample {
    /// Legacy root pass totals partitioned by the submitted frame's view. The
    /// first bucket is shared work; empty views remain explicit. These sums are
    /// diagnostics, not additive hardware utilization or a frame-time estimate.
    pub fn view_pass_ms(&self) -> Vec<(Option<u32>, Vec<(&'static str, f64)>)> {
        std::iter::once(None).chain((0..self.views).map(Some)).map(|view| {
            let mut passes: Vec<(&'static str, f64)> = Vec::new();
            for interval in self.intervals.iter().filter(|i| i.parent.is_none() && i.view_index == view) {
                let ms = interval.end_ms - interval.start_ms;
                match passes.iter_mut().find(|(name, _)| *name == interval.name) {
                    Some(entry) => entry.1 += ms,
                    None => passes.push((interval.name, ms)),
                }
            }
            (view, passes)
        }).collect()
    }
}

#[derive(Debug, Default)]
pub struct GpuProfileDrain {
    pub samples: Vec<GpuProfileSample>,
    pub pending: u32,
    /// Cumulative frames skipped because all four readback slots were occupied.
    pub dropped: u64,
    /// Cumulative mapping, polling or malformed timestamp failures.
    pub errors: u64,
    pub enabled: bool,
    pub supported: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Context {
    pub frame_id: u64,
    pub render_sequence: u64,
    pub output_size: [u32; 2],
    pub views: u32,
}

/// Query indices are explicit: an envelope aliases its children's endpoints
/// without another timestamp write or an extra render pass.
#[derive(Clone, Debug)]
pub(crate) struct PassSpec {
    name: &'static str,
    queries: [u32; 2],
    parent: Option<usize>,
    view_index: Option<u32>,
}

pub(crate) struct Envelope {
    pub name: &'static str,
    /// Beginning query indices of the depth, material and illumination pairs.
    pub children: [u32; 3],
}

/// Each range is captured while encoding that view, not inferred when an
/// asynchronous result arrives. Ranges exclude all shared work.
pub(crate) fn describe_passes(
    names: &[&'static str],
    views: &[(u32, std::ops::Range<usize>)],
    envelopes: &[Envelope],
) -> Result<Vec<PassSpec>, ()> {
    let mut tags = vec![None; names.len()];
    for (view, range) in views {
        if range.start > range.end || range.end > names.len() { return Err(()); }
        for tag in &mut tags[range.clone()] {
            if tag.replace(*view).is_some() { return Err(()); }
        }
    }
    let mut owners = vec![None; names.len()];
    for (index, envelope) in envelopes.iter().enumerate() {
        let [a, b, c] = envelope.children;
        if envelope.name.is_empty() || a % 2 != 0 || b % 2 != 0 || c % 2 != 0
            || !(a < b && b < c) || c as usize / 2 >= names.len()
        { return Err(()); }
        for query in envelope.children {
            let pair = query as usize / 2;
            if owners[pair].replace(index).is_some() || tags[pair] != tags[a as usize / 2] { return Err(()); }
        }
    }
    let mut specs = Vec::with_capacity(names.len() + envelopes.len());
    let mut parents = vec![None; envelopes.len()];
    for (pair, &name) in names.iter().enumerate() {
        let parent = if let Some(owner) = owners[pair] {
            if parents[owner].is_none() {
                let envelope = &envelopes[owner];
                parents[owner] = Some(specs.len());
                specs.push(PassSpec { name: envelope.name,
                    queries: [envelope.children[0], envelope.children[2] + 1],
                    parent: None, view_index: tags[pair] });
            }
            parents[owner]
        } else { None };
        specs.push(PassSpec { name, queries: [pair as u32 * 2, pair as u32 * 2 + 1], parent, view_index: tags[pair] });
    }
    Ok(specs)
}

fn valid_specs(context: Context, specs: &[PassSpec], query_count: u32) -> bool {
    if query_count == 0 || query_count % 2 != 0 || specs.is_empty() { return false; }
    let mut covered = vec![false; query_count as usize / 2];
    for (index, spec) in specs.iter().enumerate() {
        let [start, end] = spec.queries;
        if spec.name.is_empty() || start % 2 != 0 || end % 2 != 1 || start >= end || end >= query_count
            || spec.view_index.is_some_and(|view| view >= context.views)
            || spec.parent.is_some_and(|parent| parent >= index || specs[parent].view_index != spec.view_index)
        { return false; }
        if end == start + 1 { covered[start as usize / 2] = true; }
    }
    covered.into_iter().all(|value| value)
}

struct Pending {
    context: Context,
    specs: Vec<PassSpec>,
    query_count: u32,
    period_ns: f64,
    status: Arc<AtomicU8>,
}

struct Slot {
    read: wgpu::Buffer,
    pending: Option<Pending>,
}

/// Owns a clone of its destination, so no profiler lock spans queue submission.
pub(crate) struct Ticket {
    pub read: wgpu::Buffer,
    pub query_count: u32,
    status: Arc<AtomicU8>,
}

impl Ticket {
    /// Call only after submitting the encoder containing the resolve/copy.
    pub fn submitted(self) {
        let status = self.status;
        self.read.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            status.store(if result.is_ok() { READY } else { FAILED }, Ordering::Release);
        });
    }
}

pub(crate) struct Profiler {
    enabled: bool,
    supported: bool,
    capacity: u32,
    slots: Vec<Slot>,
    dropped: u64,
    errors: u64,
}

impl Profiler {
    pub fn new(supported: bool, capacity: u32) -> Self {
        Self { enabled: false, supported, capacity, slots: Vec::new(), dropped: 0, errors: 0 }
    }

    pub fn configure(&mut self, gpu: &Gpu, enabled: bool) -> Result<(), String> {
        if enabled && !self.supported {
            return Err("asynchronous GPU profiling requires timestamp queries".into());
        }
        if enabled && self.slots.is_empty() {
            self.slots = (0..RING_SIZE).map(|_| Slot {
                read: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("async GPU timestamp readback"),
                    size: u64::from(self.capacity) * 8,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }), pending: None,
            }).collect();
        }
        // Disabling preserves in-flight samples. A later drain can still collect
        // them; re-enabling neither reallocates slots nor resets counters.
        self.enabled = enabled;
        Ok(())
    }

    pub fn reserve(&mut self, context: Context, specs: &[PassSpec], query_count: u32, period_ns: f64) -> Option<Ticket> {
        if !self.enabled || query_count == 0 { return None; }
        if query_count > self.capacity || !valid_specs(context, specs, query_count) || !period_ns.is_finite() || period_ns <= 0.0 {
            self.errors += 1;
            return None;
        }
        let Some(slot) = self.slots.iter_mut().find(|s| s.pending.is_none()) else {
            self.dropped += 1;
            return None;
        };
        let status = Arc::new(AtomicU8::new(WAITING));
        slot.pending = Some(Pending { context, specs: specs.to_vec(), query_count, period_ns, status: status.clone() });
        Some(Ticket { read: slot.read.clone(), query_count, status })
    }

    /// Poll dispatches callbacks already ready on the device. It never waits for
    /// a submission; a slow GPU leaves the corresponding slots pending.
    pub fn drain(&mut self, gpu: &Gpu) -> GpuProfileDrain {
        if !self.slots.is_empty() && gpu.device.poll(wgpu::PollType::Poll).is_err() { self.errors += 1; }
        let mut samples = Vec::with_capacity(RING_SIZE);
        for slot in &mut self.slots {
            let Some(pending) = slot.pending.as_ref() else { continue; };
            let status = pending.status.load(Ordering::Acquire);
            if status == WAITING { continue; }
            let pending = slot.pending.take().unwrap();
            if status == FAILED {
                self.errors += 1;
                continue;
            }
            let result = {
                match slot.read.slice(..).get_mapped_range() {
                    Ok(bytes) => decode(&pending, bytemuck::cast_slice(&bytes)),
                    Err(_) => Err(()),
                }
            };
            slot.read.unmap();
            match result {
                Ok(sample) => samples.push(sample),
                Err(()) => self.errors += 1,
            }
        }
        samples.sort_unstable_by_key(|s| s.render_sequence);
        GpuProfileDrain { samples, pending: self.slots.iter().filter(|s| s.pending.is_some()).count() as u32,
            dropped: self.dropped, errors: self.errors, enabled: self.enabled, supported: self.supported }
    }
}

fn decode(pending: &Pending, timestamps: &[u64]) -> Result<GpuProfileSample, ()> {
    decode_timestamps(pending.context, &pending.specs, pending.query_count, pending.period_ns, timestamps)
}

/// Shared by synchronous diagnostics and the delayed ring: both retain exactly
/// the same root totals and authored timing hierarchy.
pub(crate) fn decode_timestamps(context: Context, specs: &[PassSpec], query_count: u32,
    period_ns: f64, timestamps: &[u64]) -> Result<GpuProfileSample, ()>
{
    if timestamps.len() < query_count as usize || !valid_specs(context, specs, query_count)
        || !period_ns.is_finite() || period_ns <= 0.0 { return Err(()); }
    let mut gpu_ms: Vec<(&'static str, f64)> = Vec::new();
    let mut instrumented_gpu_ms = 0.0;
    let mut first = u64::MAX;
    let mut last = 0;
    let scale = period_ns / 1e6;
    for spec in specs {
        let start = timestamps[spec.queries[0] as usize];
        let end = timestamps[spec.queries[1] as usize];
        // Passes can overlap on some backends. Do not infer serialized GPU
        // work from their order, but reject an internally backwards interval.
        if end < start { return Err(()); }
        // A pair left at zero was never written (some Metal passes skip
        // encoder-level timestamps). It must not pull the span back to the
        // GPU clock's origin: that read as days of covered time.
        if end != 0 {
            first = first.min(start);
            last = last.max(end);
        }
        let ms = (end - start) as f64 * scale;
        if !ms.is_finite() { return Err(()); }
        if spec.parent.is_none() {
            instrumented_gpu_ms += ms;
            match gpu_ms.iter_mut().find(|(name, _)| *name == spec.name) {
                Some(entry) => entry.1 += ms,
                None => gpu_ms.push((spec.name, ms)),
            }
        }
    }
    if last == 0 { first = 0; }
    let covered_interval_ms = (last - first) as f64 * scale;
    if !covered_interval_ms.is_finite() || !instrumented_gpu_ms.is_finite() { return Err(()); }
    // Subtract the integer origin before converting to f64. Device uptime
    // therefore cannot erase short intervals through absolute float rounding.
    let intervals: Vec<_> = specs.iter().map(|spec| GpuPassInterval {
        name: spec.name, start_ms: timestamps[spec.queries[0] as usize].saturating_sub(first) as f64 * scale,
        end_ms: timestamps[spec.queries[1] as usize].saturating_sub(first) as f64 * scale,
        parent: spec.parent, view_index: spec.view_index,
    }).collect();
    for interval in &intervals {
        if let Some(parent) = interval.parent {
            let scope = &intervals[parent];
            if interval.end_ms == 0.0 || scope.end_ms == 0.0 { continue; }
            if scope.start_ms > interval.start_ms || scope.end_ms < interval.end_ms { return Err(()); }
        }
    }
    let covered_union_ms = interval_union_ms(intervals.iter().map(|i| (i.start_ms, i.end_ms))).ok_or(())?;
    Ok(GpuProfileSample { frame_id: context.frame_id, render_sequence: context.render_sequence,
        gpu_ms, instrumented_gpu_ms, covered_interval_ms, covered_union_ms, intervals,
        output_size: context.output_size, views: context.views })
}

/// Exact union of finite, nonnegative timestamp intervals. A valid empty list
/// has zero coverage; callers decide whether missing instrumentation is valid.
pub fn interval_union_ms(intervals: impl IntoIterator<Item = (f64, f64)>) -> Option<f64> {
    let mut values: Vec<_> = intervals.into_iter().collect();
    if values.iter().any(|&(start, end)| !start.is_finite() || !end.is_finite() || start < 0.0 || end < start) {
        return None;
    }
    values.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    let mut total = 0.0;
    let mut span: Option<(f64, f64)> = None;
    for (start, end) in values {
        match span {
            Some((lo, hi)) if start <= hi => span = Some((lo, hi.max(end))),
            Some((lo, hi)) => { total += hi - lo; span = Some((start, end)); }
            None => span = Some((start, end)),
        }
    }
    if let Some((lo, hi)) = span { total += hi - lo; }
    total.is_finite().then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending(names: &[&'static str]) -> Pending {
        Pending { context: Context { frame_id: 97, render_sequence: 4, output_size: [3840,2160], views: 4 },
            specs: describe_passes(names, &[], &[]).unwrap(), query_count: names.len() as u32 * 2,
            period_ns: 2.0, status: Arc::new(AtomicU8::new(WAITING)) }
    }
    #[test]
    fn absent_queries_and_native_without_aggregate_do_not_invent_scopes() {
        // Unsupported timestamps still encode view ranges, all of them empty.
        let specs = describe_passes(&[], &[(0, 0..0), (1, 0..0), (2, 0..0), (3, 0..0)], &[]).unwrap();
        assert!(specs.is_empty());
        assert!(describe_passes(&[], &[], &[]).unwrap().is_empty());
        let mut profiler = Profiler::new(false, 400);
        profiler.enabled = true; // Exercise reserve's no-query path without a device.
        assert!(profiler.reserve(pending(&[]).context, &specs, 0, 1.0).is_none());
        assert_eq!((profiler.dropped, profiler.errors), (0, 0));

        let names = ["shade", "meadow placement", "meadow render", "meadow composite", "tonemap"];
        let mut p = pending(&names);
        p.context.views = 1;
        p.specs = describe_passes(&names, &[(0, 0..names.len())], &[]).unwrap();
        let measured = decode(&p, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]).unwrap();
        assert_eq!(measured.intervals.len(), names.len());
        assert!(measured.intervals.iter().all(|i| i.parent.is_none() && i.view_index == Some(0)));
        assert!(!measured.gpu_ms.iter().any(|(name, _)| name.contains("aggregate")));
        assert!(measured.view_pass_ms()[0].1.is_empty(), "no fabricated shared work");
    }
    #[test]
    fn unwritten_pairs_do_not_stretch_the_covered_span() {
        // A Metal pass without encoder timestamps leaves its pair at zero.
        let p = pending(&["sky", "skipped", "shade"]);
        let up = 412_000_000_000_000u64; // a GPU clock days after boot
        let sample = decode(&p, &[up, up + 1000, 0, 0, up + 1000, up + 4000]).unwrap();
        assert_eq!(sample.covered_interval_ms, 4000.0 * 2.0 / 1e6);
        assert_eq!(sample.covered_union_ms, 4000.0 * 2.0 / 1e6);
        assert_eq!((sample.intervals[1].start_ms, sample.intervals[1].end_ms), (0.0, 0.0));
    }
    #[test]
    fn submitted_view_tags_preserve_shared_work_and_survive_next_frame_layout() {
        let names = ["headlight raster", "near sun raster", "shade", "near sun raster", "shade",
            "near sun raster", "shade", "near sun raster", "shade"];
        let mut ranges = vec![(0, 1..3), (1, 3..5), (2, 5..7), (3, 7..9)];
        let mut p = pending(&names);
        p.specs = describe_passes(&names, &ranges, &[]).unwrap();
        // The next encode can have another layout before this readback arrives.
        ranges.clear(); ranges.push((0, 0..names.len()));
        let _next = describe_passes(&names, &ranges, &[]).unwrap();
        let times: Vec<_> = (0..names.len() as u64).flat_map(|i| [100 + i * 20, 110 + i * 20]).collect();
        let sample = decode(&p, &times).unwrap();
        assert_eq!(sample.intervals[0].view_index, None);
        for view in 0..4 {
            assert_eq!(sample.intervals[1 + view * 2].name, "near sun raster");
            assert_eq!(sample.intervals[1 + view * 2].view_index, Some(view as u32));
            assert_eq!(sample.intervals[2 + view * 2].view_index, Some(view as u32));
        }
        let buckets = sample.view_pass_ms();
        assert_eq!(buckets.len(), 5);
        assert_eq!(buckets[0].0, None);
        assert_eq!(buckets[0].1[0].0, "headlight raster");
        let bucket_sum: f64 = buckets.iter().flat_map(|(_, passes)| passes.iter().map(|(_, ms)| ms)).sum();
        assert!((bucket_sum - sample.instrumented_gpu_ms).abs() < 1e-12);
        assert_eq!((sample.frame_id, sample.render_sequence), (97, 4));
    }

    #[test]
    fn aggregate_children_preserve_legacy_envelope_and_sync_async_totals() {
        // Physical allocation order need not be execution order: merge is
        // allocated before material/illumination in the meadow timing array.
        let names = ["shared sun", "meadow aggregate depth", "meadow merge",
            "meadow aggregate material", "meadow aggregate illumination"];
        let mut p = pending(&names);
        p.specs = describe_passes(&names, &[(2, 1..5)], &[Envelope {
            name: "meadow aggregate render", children: [2, 6, 8],
        }]).unwrap();
        let times = [0, 5, 10, 20, 75, 80, 25, 45, 50, 70];
        let delayed = decode(&p, &times).unwrap();
        let sync = decode_timestamps(p.context, &p.specs, p.query_count, p.period_ns, &times).unwrap();
        let legacy = decode(&pending(&["shared sun", "meadow aggregate render", "meadow merge"]),
            &[0, 5, 10, 70, 75, 80]).unwrap();
        assert_eq!(delayed.gpu_ms, legacy.gpu_ms);
        assert_eq!(delayed.gpu_ms, sync.gpu_ms);
        assert_eq!(delayed.intervals, sync.intervals);
        assert_eq!(delayed.instrumented_gpu_ms, legacy.instrumented_gpu_ms);
        assert_eq!(delayed.covered_interval_ms, legacy.covered_interval_ms);
        assert_eq!(delayed.covered_union_ms, legacy.covered_union_ms);
        let parent = delayed.intervals.iter().position(|i| i.name == "meadow aggregate render").unwrap();
        let children: Vec<_> = delayed.intervals.iter().filter(|i| i.parent == Some(parent)).collect();
        assert_eq!(children.len(), 3);
        assert!(children.iter().all(|i| i.view_index == Some(2)));
        let child_sum: f64 = children.iter().map(|i| i.end_ms - i.start_ms).sum();
        assert!(child_sum < delayed.gpu_ms[1].1, "parent retains the original inter-pass gaps");
    }

    #[test]
    fn authored_layout_rejects_bad_ranges_queries_parents_and_view_indices() {
        let names = ["depth", "material", "illumination"];
        assert!(describe_passes(&names, &[(0, 0..2), (1, 1..3)], &[]).is_err());
        assert!(describe_passes(&names, &[(0, 0..4)], &[]).is_err());
        assert!(describe_passes(&names, &[(0, 0..1), (1, 1..3)], &[Envelope {
            name: "aggregate", children: [0, 2, 4],
        }]).is_err());
        for queries in [[0, 2, 6], [1, 2, 4], [0, 0, 4]] {
            assert!(describe_passes(&names, &[], &[Envelope { name: "aggregate", children: queries }]).is_err());
        }
        for kind in 0..4 {
            let mut p = pending(&names);
            match kind {
                0 => p.specs[0].view_index = Some(4),
                1 => p.specs[0].queries[1] = 7,
                2 => p.specs[0].parent = Some(0),
                _ => { p.specs[1].parent = Some(0); p.specs[1].view_index = Some(1); },
            }
            assert!(decode(&p, &[0, 5, 10, 15, 20, 25]).is_err());
        }
        let mut p = pending(&names);
        p.specs = describe_passes(&names, &[], &[Envelope { name: "aggregate", children: [0, 2, 4] }]).unwrap();
        assert!(decode(&p, &[10, 20, 0, 40, 50, 60]).is_err(), "child outside authored parent is invalid");
    }
    #[test]
    fn gpu_profile_aggregates_views_but_keeps_real_interval_and_ids() {
        let sample = decode(&pending(&["shade","tonemap","shade"]), &[100,200,240,260,400,550]).unwrap();
        assert_eq!(sample.gpu_ms.len(), 2);
        assert_eq!(sample.gpu_ms[0].0, "shade");
        assert!((sample.gpu_ms[0].1 - 0.0005).abs() < 1e-12);
        assert!((sample.gpu_ms[1].1 - 0.00004).abs() < 1e-12);
        assert!((sample.instrumented_gpu_ms - 0.00054).abs() < 1e-12);
        assert!((sample.covered_interval_ms - 0.0009).abs() < 1e-12);
        assert_eq!((sample.frame_id,sample.render_sequence,sample.views),(97,4,4));
        assert_eq!(sample.output_size,[3840,2160]);
    }
    #[test]
    fn gpu_profile_rejects_invalid_traces_instead_of_zero_timings() {
        for values in [&[10,9,11,12][..], &[10,15,16,14], &[10,15]] {
            assert!(decode(&pending(&["a","b"]),values).is_err());
        }
        let mut p=pending(&["a"]); p.period_ns=f64::NAN;
        assert!(decode(&p,&[10,15]).is_err());
    }
    #[test]
    fn gpu_profile_ignores_unused_readback_tail() {
        let p=pending(&["a"]);
        let a=decode(&p,&[100,150]).unwrap();
        let b=decode(&p,&[100,150,u64::MAX,0]).unwrap();
        assert_eq!(a.covered_interval_ms,b.covered_interval_ms);
        assert_eq!(a.gpu_ms,b.gpu_ms);
    }
    #[test]
    fn gpu_profile_overlapping_intervals_are_not_invented_as_serial_work() {
        let sample=decode(&pending(&["a","b"]),&[10,30,20,40]).unwrap();
        assert!((sample.instrumented_gpu_ms - 0.00008).abs()<1e-12);
        assert!((sample.covered_interval_ms - 0.00006).abs()<1e-12);
        assert!((sample.covered_union_ms - 0.00006).abs()<1e-12);
    }
    #[test]
    fn union_keeps_gaps_nested_scopes_and_large_integer_origins_distinct() {
        let origin = u64::MAX - 1_000;
        let sample = decode(&pending(&["parent", "child", "later"]),
            &[origin, origin + 100, origin + 20, origin + 80, origin + 150, origin + 180]).unwrap();
        assert!((sample.covered_union_ms - 0.00026).abs() < 1e-12);
        assert!((sample.covered_interval_ms - 0.00036).abs() < 1e-12);
        assert!((sample.instrumented_gpu_ms - 0.00038).abs() < 1e-12);
        assert_eq!(sample.intervals[0].start_ms, 0.0);
        assert_eq!(sample.intervals[1].parent, None, "overlap must not invent an instrumentation hierarchy");
        for invalid in [(f64::NAN, 1.0), (-1.0, 1.0), (2.0, 1.0), (0.0, f64::INFINITY)] {
            assert_eq!(interval_union_ms([invalid]), None);
        }
    }
}
