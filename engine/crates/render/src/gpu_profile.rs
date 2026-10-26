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
    /// Current renderer timestamps are individual passes, so this is None.
    pub parent: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct GpuProfileSample {
    pub frame_id: u64,
    pub render_sequence: u64,
    /// Same-named passes across all views are summed.
    pub gpu_ms: Vec<(&'static str, f64)>,
    /// Sum of measured pass intervals only; excludes uninstrumented gaps.
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

struct Pending {
    context: Context,
    names: Vec<&'static str>,
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

    pub fn reserve(&mut self, context: Context, names: &[&'static str], query_count: u32, period_ns: f64) -> Option<Ticket> {
        if !self.enabled || query_count == 0 { return None; }
        if query_count > self.capacity || query_count as usize != names.len() * 2 || !period_ns.is_finite() || period_ns <= 0.0 {
            self.errors += 1;
            return None;
        }
        let Some(slot) = self.slots.iter_mut().find(|s| s.pending.is_none()) else {
            self.dropped += 1;
            return None;
        };
        let status = Arc::new(AtomicU8::new(WAITING));
        slot.pending = Some(Pending { context, names: names.to_vec(), period_ns, status: status.clone() });
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
    let count = pending.names.len() * 2;
    if count == 0 || timestamps.len() < count || !pending.period_ns.is_finite() || pending.period_ns <= 0.0 { return Err(()); }
    let mut gpu_ms: Vec<(&'static str, f64)> = Vec::new();
    let mut instrumented_gpu_ms = 0.0;
    let mut first = u64::MAX;
    let mut last = 0;
    let scale = pending.period_ns / 1e6;
    for (k, name) in pending.names.iter().enumerate() {
        let start = timestamps[k * 2];
        let end = timestamps[k * 2 + 1];
        // Passes can overlap on some backends. Do not infer serialized GPU
        // work from their order, but reject an internally backwards interval.
        if end < start { return Err(()); }
        first = first.min(start);
        last = last.max(end);
        let ms = (end - start) as f64 * scale;
        if !ms.is_finite() { return Err(()); }
        instrumented_gpu_ms += ms;
        match gpu_ms.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 += ms,
            None => gpu_ms.push((name, ms)),
        }
    }
    let covered_interval_ms = (last - first) as f64 * scale;
    if !covered_interval_ms.is_finite() || !instrumented_gpu_ms.is_finite() { return Err(()); }
    // Subtract the integer origin before converting to f64. Device uptime
    // therefore cannot erase short intervals through absolute float rounding.
    let intervals: Vec<_> = pending.names.iter().enumerate().map(|(k, &name)| GpuPassInterval {
        name, start_ms: (timestamps[k * 2] - first) as f64 * scale,
        end_ms: (timestamps[k * 2 + 1] - first) as f64 * scale, parent: None,
    }).collect();
    let covered_union_ms = interval_union_ms(intervals.iter().map(|i| (i.start_ms, i.end_ms))).ok_or(())?;
    Ok(GpuProfileSample { frame_id: pending.context.frame_id, render_sequence: pending.context.render_sequence,
        gpu_ms, instrumented_gpu_ms, covered_interval_ms, covered_union_ms, intervals,
        output_size: pending.context.output_size, views: pending.context.views })
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
            names: names.to_vec(), period_ns: 2.0, status: Arc::new(AtomicU8::new(WAITING)) }
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
