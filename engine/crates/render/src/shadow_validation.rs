//! Bounded, nonblocking validation of submitted foliage-page cluster counters.
//!
//! Hold the caller's State lock from page planning through encode/submission.
//! When available() is false, defer new page updates; never submit an update
//! without its validation ticket. Call submitted() only after queue.submit().
//! Successful submission permits provisional cache use, not proof that the
//! counters fit: a later drain reports any overflow as a persistent error.

use crate::Gpu;
use std::sync::mpsc::{self, Receiver, TryRecvError};

const MAX_PENDING: usize = 4;
/// Layer label of the legacy far-sun map's counter in a batch.
pub(crate) const FAR_SUN: u32 = u32::MAX;
const MAX_PAGES_PER_BATCH: usize = 8;
const COUNTER_STRIDE: usize = 16;

struct Pending {
    read: wgpu::Buffer,
    pages: Vec<(u32, u32)>,
    result: Receiver<Result<(), String>>,
}

/// One encoded batch. Keep at most one unsubmitted ticket per locked State.
/// Dropping an unsubmitted ticket is safe only if its encoder is not submitted.
#[must_use = "submit this validation ticket after submitting its encoder"]
pub(crate) struct Ticket {
    read: wgpu::Buffer,
    pages: Vec<(u32, u32)>,
}

#[derive(Default)]
pub(crate) struct State {
    pending: Vec<Pending>,
    // A caller retry must not silently resume using a page that overflowed.
    failure: Option<String>,
}

fn has_capacity(pending: usize) -> bool {
    pending < MAX_PENDING
}

fn validate(pages: &[(u32, u32)], bytes: &[u8]) -> Result<(), String> {
    if pages.is_empty() || pages.len() > MAX_PAGES_PER_BATCH {
        return Err(format!("foliage page validation has invalid batch size {}", pages.len()));
    }
    let expected = pages.len() * COUNTER_STRIDE;
    if bytes.len() != expected {
        return Err(format!("foliage page validation has malformed counter data: {} bytes, expected {expected}", bytes.len()));
    }
    for (index, &(layer, capacity)) in pages.iter().enumerate() {
        let offset = index * COUNTER_STRIDE;
        // Read bytes directly; mapped buffers need no host alignment assumption.
        let count = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if count > capacity {
            let what = if layer == FAR_SUN { "far sun map".to_owned() } else { format!("foliage page {layer}") };
            return Err(format!("{what} cluster capacity exceeded asynchronously: {count}>{capacity}; submitted cache content is invalid"));
        }
    }
    Ok(())
}

impl State {
    pub fn available(&self) -> bool {
        self.failure.is_none() && has_capacity(self.pending.len())
    }

    /// Full queues return None without encoding anything. The caller must
    /// decide the update budget before rendering pages, not discard validation.
    pub fn encode(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder,
        pages: &[(u32, u32, &wgpu::Buffer)]) -> Option<Ticket>
    {
        assert!(pages.len() <= MAX_PAGES_PER_BATCH, "foliage validation batch exceeds eight page updates");
        if pages.is_empty() || !self.available() {
            return None;
        }
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("async foliage page counter validation"),
            size: (pages.len() * COUNTER_STRIDE) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for (index, &(_, _, source)) in pages.iter().enumerate() {
            encoder.copy_buffer_to_buffer(source, 0, &read,
                (index * COUNTER_STRIDE) as u64, COUNTER_STRIDE as u64);
        }
        Some(Ticket { read, pages: pages.iter().map(|&(layer, capacity, _)| (layer, capacity)).collect() })
    }

    /// Registers every submitted batch. Queue saturation is handled before
    /// planning, so reaching it here is a caller contract violation.
    pub fn submitted(&mut self, ticket: Ticket) {
        assert!(self.available(), "foliage page validation submitted without a reserved free batch");
        let (send, result) = mpsc::channel();
        ticket.read.slice(..).map_async(wgpu::MapMode::Read, move |mapped| {
            let _ = send.send(mapped.map_err(|error| format!("foliage page validation mapping failed: {error}")));
        });
        self.pending.push(Pending { read: ticket.read, pages: ticket.pages, result });
    }

    /// Poll dispatches only callbacks already ready. No wait, blocking receive,
    /// dropped batch, or eviction of pending work is permitted on this path.
    pub fn drain(&mut self, gpu: &Gpu) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if self.pending.is_empty() {
            return Ok(());
        }
        if let Err(error) = gpu.device.poll(wgpu::PollType::Poll) {
            return self.fail(format!("foliage page validation device poll failed: {error}"));
        }
        let mut index = 0;
        while index < self.pending.len() {
            let completion = match self.pending[index].result.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => { index += 1; continue; }
                Err(TryRecvError::Disconnected) => Err("foliage page validation mapping callback disconnected".into()),
            };
            let pending = self.pending.remove(index);
            let result = completion.and_then(|()| {
                let bytes = pending.read.slice(..).get_mapped_range()
                    .map_err(|error| format!("foliage page validation mapped range failed: {error}"))?;
                validate(&pending.pages, &bytes)
            });
            // Drop the mapped view inside the closure before unmapping, even
            // when validation fails. Other pending batches remain retained.
            pending.read.unmap();
            if let Err(error) = result {
                return self.fail(error);
            }
        }
        Ok(())
    }

    fn fail(&mut self, error: String) -> Result<(), String> {
        Err(self.failure.get_or_insert(error).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters(counts: &[u32]) -> Vec<u8> {
        let mut bytes = vec![0xff; counts.len() * COUNTER_STRIDE];
        for (index, count) in counts.iter().enumerate() {
            bytes[index * COUNTER_STRIDE..index * COUNTER_STRIDE + 4].copy_from_slice(&count.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn page_counters_use_sixteen_byte_stride_and_allow_exact_capacity() {
        // Triangle/vertex/diagnostic counters are deliberately u32::MAX; only
        // the first word in each page's four-word record is a cluster count.
        assert!(validate(&[(7, 0), (201, 35), (255, u32::MAX)], &counters(&[0, 35, u32::MAX])).is_ok());
        let error = validate(&[(7, 0), (201, 35)], &counters(&[0, 36])).unwrap_err();
        assert!(error.contains("page 201"));
        assert!(error.contains("36>35"));
        assert!(error.contains("cache content is invalid"));
    }

    #[test]
    fn malformed_page_counter_batches_fail_closed() {
        for len in [0, 4, 15, 17, 32] {
            assert!(validate(&[(2, 10)], &vec![0; len]).is_err(), "accepted {len} bytes for one page");
        }
        assert!(validate(&[], &[]).is_err());
        assert!(validate(&[(2, 10); MAX_PAGES_PER_BATCH + 1], &counters(&[0; MAX_PAGES_PER_BATCH + 1])).is_err());
        assert!(validate(&[(2, 10); MAX_PAGES_PER_BATCH], &counters(&[10; MAX_PAGES_PER_BATCH])).is_ok());
    }

    #[test]
    fn validation_failure_stays_fatal_on_retry() {
        let mut state = State::default();
        assert!(state.available());
        assert_eq!(state.fail("bad page".into()), Err("bad page".into()));
        assert!(!state.available());
        assert_eq!(state.fail("later poll failure".into()), Err("bad page".into()));
        assert_eq!(state.failure.as_deref(), Some("bad page"));
    }
}
