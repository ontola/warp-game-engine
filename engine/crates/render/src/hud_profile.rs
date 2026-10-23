//! Optional HUD pass timing for bounded, hidden validation runs.
//!
//! Construct only when profiling is requested. `writes` and `resolve` record
//! GPU commands without mapping, polling or waiting. Call `read` once after
//! submitting all measured frames; its bounded wait is deliberately outside
//! normal play and outside the frame-wall timing interval.

use crate::Gpu;
use std::{sync::mpsc, time::Duration};

/// At most about one MiB of GPU resolve storage per profiling run.
pub const MAX_HUD_PROFILE_SAMPLES: u32 = 4096;
const STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
const PAIR_BYTES: u64 = 16;

pub struct HudProfiler {
    query: wgpu::QuerySet,
    resolved: wgpu::Buffer,
    capacity: u32,
    used: u32,
}

impl HudProfiler {
    /// Returns None only when the device cannot supply timestamp queries.
    /// Capacity is capped at MAX_HUD_PROFILE_SAMPLES; exceeding the reserved
    /// capacity during recording is an explicit error, never silent truncation.
    /// Zero capacity is allowed but every attempted resolve will fail.
    pub fn new(gpu: &Gpu, capacity: u32) -> Option<Self> {
        if !gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let capacity = capacity.min(MAX_HUD_PROFILE_SAMPLES);
        let query = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("HUD profile timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolved = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("HUD profile resolved samples"),
            size: slot_span(capacity).max(PAIR_BYTES),
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Some(Self { query, resolved, capacity, used: 0 })
    }

    /// Attach to the HUD-only render pass, after the scene presentation pass.
    pub fn writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.query,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    /// Call after ending the corresponding HUD pass and before submitting its
    /// encoder. Submit that encoder before recording another measured pass:
    /// the two query indices are reused, with results preserved in distinct
    /// aligned slots. Do not call when no timestamped HUD pass was recorded.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) -> Result<(), String> {
        if self.used >= self.capacity {
            return Err(format!("HUD profile capacity {} exhausted", self.capacity));
        }
        encoder.resolve_query_set(&self.query, 0..2, &self.resolved, u64::from(self.used) * STRIDE);
        self.used += 1;
        Ok(())
    }

    /// Read all recorded samples in submission order, in milliseconds. This
    /// performs one copy/map/wait at the end of a validation run. Missing,
    /// reversed, zero-duration or invalid timestamps are errors, not zeros.
    pub fn read(&self, gpu: &Gpu) -> Result<Vec<f64>, String> {
        if self.used == 0 {
            return Ok(Vec::new());
        }
        let size = slot_span(self.used);
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("HUD profile readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("HUD profile final readback"),
        });
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &readback, 0, size);
        let submitted = gpu.queue.submit([encoder.finish()]);
        let (sender, receiver) = mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        gpu.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submitted),
            timeout: Some(Duration::from_secs(30)),
        }).map_err(|e| format!("HUD profile GPU wait failed: {e:?}"))?;
        receiver.try_recv().map_err(|e| format!("HUD profile mapping callback missing: {e}"))?
            .map_err(|e| format!("HUD profile mapping failed: {e:?}"))?;
        let samples = (|| {
            let bytes = readback.slice(..).get_mapped_range()
                .map_err(|e| format!("HUD profile mapped range failed: {e:?}"))?;
            decode_samples(&bytes, self.used, f64::from(gpu.queue.get_timestamp_period()))
        })();
        readback.unmap();
        samples
    }
}

fn slot_span(count: u32) -> u64 {
    if count == 0 { 0 } else { u64::from(count - 1) * STRIDE + PAIR_BYTES }
}

fn decode_samples(bytes: &[u8], count: u32, period_ns: f64) -> Result<Vec<f64>, String> {
    if !period_ns.is_finite() || period_ns <= 0.0 {
        return Err(format!("HUD profile invalid timestamp period: {period_ns}"));
    }
    if bytes.len() as u64 != slot_span(count) {
        return Err(format!("HUD profile readback size mismatch: {} bytes for {count} samples", bytes.len()));
    }
    let mut samples = Vec::with_capacity(count as usize);
    for index in 0..count {
        let offset = (u64::from(index) * STRIDE) as usize;
        // The span check above guarantees both complete u64 values per slot.
        let begin = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        let end = u64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
        let ticks = end.checked_sub(begin)
            .filter(|ticks| *ticks > 0)
            .ok_or_else(|| format!("HUD profile sample {index} has non-increasing timestamps: {begin} -> {end}"))?;
        let ms = ticks as f64 * period_ns / 1_000_000.0;
        if !ms.is_finite() || ms <= 0.0 {
            return Err(format!("HUD profile sample {index} has invalid duration: {ms}"));
        }
        samples.push(ms);
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(pairs: &[(u64, u64)]) -> Vec<u8> {
        // Poison alignment padding so reading packed pairs instead of aligned
        // slots cannot accidentally yield plausible zero-duration samples.
        let mut bytes = vec![0xff; slot_span(pairs.len() as u32) as usize];
        for (index, (begin, end)) in pairs.iter().enumerate() {
            let offset = index * STRIDE as usize;
            bytes[offset..offset + 8].copy_from_slice(&begin.to_le_bytes());
            bytes[offset + 8..offset + 16].copy_from_slice(&end.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn aligned_slots_preserve_short_durations_and_order() {
        let bytes = encode(&[(100, 110), (900, 920)]);
        assert_eq!(bytes.len(), 272);
        let values = decode_samples(&bytes, 2, 2.5).unwrap();
        assert!((values[0] - 0.000025).abs() < 1e-12);
        assert!((values[1] - 0.000050).abs() < 1e-12);
        assert_eq!(slot_span(MAX_HUD_PROFILE_SAMPLES), 1_048_336);
    }

    #[test]
    fn invalid_timestamps_never_become_zero_timings() {
        for pair in [(42, 41), (42, 42), (0, 0)] {
            assert!(decode_samples(&encode(&[pair]), 1, 1.0).is_err());
        }
        for period in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(decode_samples(&encode(&[(1, 2)]), 1, period).is_err());
        }
        assert!(decode_samples(&[0; 15], 1, 1.0).is_err());
        assert!(decode_samples(&encode(&[(1, 2)]), 2, 1.0).is_err());
    }

    #[test]
    fn subtracts_large_timestamps_before_float_conversion() {
        // Conversion of the absolute timestamps to f64 would lose this tick.
        let values = decode_samples(&encode(&[(u64::MAX - 1, u64::MAX)]), 1, 1.0).unwrap();
        assert_eq!(values, vec![0.000001]);
    }
}
