//! Explicit pre-reconstruction aggregate diagnostics. This module does no I/O.
//! Texture handles retain existing allocations; staging buffers exist only for
//! a requested capture and are released when its bytes have been copied out.

use crate::Gpu;

#[derive(Debug)]
pub struct MeadowRawLayer {
    pub name: &'static str,
    pub format: &'static str,
    pub size: [u32; 2],
    pub bytes_per_pixel: u32,
    /// Tight row stride in `data`; rows retain GPU texture order, with no flip.
    pub row_stride_bytes: u32,
    /// Padded staging stride used by the GPU copy, excluded from `data`.
    pub readback_row_stride_bytes: u32,
    /// Unconverted little-endian float/half-float bits, including alpha and
    /// signed zero. No tone mapping, exposure, color-space conversion or DLSS.
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct MeadowAggregateRaw {
    pub view: usize,
    pub size: [u32; 2],
    pub sample_prune_enabled: bool,
    pub half_resolution: bool,
    pub motion_available: bool,
    /// color: illuminated HDR + coverage; material: weighted RGB + weight;
    /// normal: weighted normal XYZ + roughness; reveal: product transmittance;
    /// depth: reversed-Z nearest aggregate/opaque depth; optional motion:
    /// previous-world minus current-world XYZ + distance, from the depth pass.
    pub layers: Vec<MeadowRawLayer>,
}

pub(super) struct Input {
    pub view: usize,
    pub size: [u32; 2],
    pub sample_prune_enabled: bool,
    pub half_resolution: bool,
    pub color: wgpu::Texture,
    pub material: wgpu::Texture,
    pub depth: wgpu::Texture,
    pub motion: Option<wgpu::Texture>,
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    tight_row: u32,
    copy_row: u32,
    bytes: u64,
}
impl Layout {
    fn new(size: [u32; 2], bytes_per_pixel: u32) -> Result<Self, String> {
        if size.contains(&0) || ![4, 8, 16].contains(&bytes_per_pixel) {
            return Err("invalid aggregate raw capture dimensions or format".into());
        }
        let tight_row = size[0].checked_mul(bytes_per_pixel).ok_or("aggregate capture row overflow")?;
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let copy_row = tight_row.checked_add(alignment - 1)
            .map(|n| n / alignment * alignment).ok_or("aggregate capture padding overflow")?;
        let bytes = u64::from(copy_row) * u64::from(size[1]);
        Ok(Self { tight_row, copy_row, bytes })
    }

    fn unpack(self, rows: u32, padded: &[u8]) -> Result<Vec<u8>, String> {
        if padded.len() as u64 != self.bytes || self.bytes != u64::from(self.copy_row) * u64::from(rows) {
            return Err("aggregate raw capture mapped size mismatch".into());
        }
        let len = usize::try_from(u64::from(self.tight_row) * u64::from(rows))
            .map_err(|_| "aggregate raw capture exceeds host address space")?;
        let mut data = Vec::with_capacity(len);
        for row in padded.chunks_exact(self.copy_row as usize) {
            data.extend_from_slice(&row[..self.tight_row as usize]);
        }
        Ok(data)
    }
}

struct Pending {
    output: usize,
    name: &'static str,
    format: &'static str,
    size: [u32; 2],
    bytes_per_pixel: u32,
    layout: Layout,
    buffer: wgpu::Buffer,
}

pub(super) fn read(gpu: &Gpu, inputs: Vec<Input>) -> Result<Vec<MeadowAggregateRaw>, String> {
    if inputs.is_empty() { return Ok(Vec::new()); }
    let mut outputs = Vec::with_capacity(inputs.len());
    let mut pending = Vec::new();
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("explicit meadow aggregate raw capture"),
    });
    for input in inputs {
        let output = outputs.len();
        outputs.push(MeadowAggregateRaw {
            view: input.view, size: input.size,
            sample_prune_enabled: input.sample_prune_enabled,
            half_resolution: input.half_resolution,
            motion_available: input.motion.is_some(), layers: Vec::new(),
        });
        let mut textures = vec![
            ("color", &input.color, 0, wgpu::TextureFormat::Rgba16Float, "Rgba16Float", 8),
            ("material", &input.material, 0, wgpu::TextureFormat::Rgba16Float, "Rgba16Float", 8),
            ("normal", &input.material, 1, wgpu::TextureFormat::Rgba16Float, "Rgba16Float", 8),
            ("reveal", &input.material, 2, wgpu::TextureFormat::Rgba16Float, "Rgba16Float", 8),
            ("depth", &input.depth, 0, wgpu::TextureFormat::Depth32Float, "Depth32Float", 4),
        ];
        if let Some(motion) = &input.motion {
            textures.push(("motion", motion, 0, wgpu::TextureFormat::Rgba32Float, "Rgba32Float", 16));
        }
        for (name, texture, layer, expected_format, format, bytes_per_pixel) in textures {
            if [texture.width(), texture.height()] != input.size
                || texture.format() != expected_format || texture.depth_or_array_layers() <= layer
                || texture.sample_count() != 1
                || !texture.usage().contains(wgpu::TextureUsages::COPY_SRC)
            {
                return Err(format!("aggregate raw capture view {} layer {name}: incompatible texture", input.view));
            }
            let layout = Layout::new(input.size, bytes_per_pixel)?;
            if layout.bytes > gpu.device.limits().max_buffer_size {
                return Err(format!("aggregate raw capture layer {name} exceeds device buffer limit"));
            }
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("meadow aggregate diagnostic staging"), size: layout.bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: if expected_format == wgpu::TextureFormat::Depth32Float {
                        wgpu::TextureAspect::DepthOnly
                    } else { wgpu::TextureAspect::All },
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer, layout: wgpu::TexelCopyBufferLayout {
                        offset: 0, bytes_per_row: Some(layout.copy_row), rows_per_image: Some(input.size[1]),
                    },
                },
                wgpu::Extent3d { width: input.size[0], height: input.size[1], depth_or_array_layers: 1 },
            );
            pending.push(Pending { output, name, format, size: input.size, bytes_per_pixel, layout, buffer });
        }
    }
    gpu.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    for (index, read) in pending.iter().enumerate() {
        let tx = tx.clone();
        read.buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send((index, result));
        });
    }
    drop(tx);
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("aggregate raw capture wait: {e}"))?;
    let mut mapped = vec![false; pending.len()];
    for _ in 0..pending.len() {
        let (index, result) = rx.try_recv().map_err(|e| format!("aggregate raw capture callback: {e}"))?;
        result.map_err(|e| format!("aggregate raw capture mapping: {e}"))?;
        mapped[index] = true;
    }
    if mapped.iter().any(|m| !m) { return Err("aggregate raw capture incomplete mapping".into()); }
    for read in pending {
        let mapped = read.buffer.slice(..).get_mapped_range().map_err(|e| format!("aggregate raw capture range: {e}"))?;
        let data = read.layout.unpack(read.size[1], &mapped)?;
        drop(mapped);
        read.buffer.unmap();
        outputs[read.output].layers.push(MeadowRawLayer {
            name: read.name, format: read.format, size: read.size,
            bytes_per_pixel: read.bytes_per_pixel,
            row_stride_bytes: read.layout.tight_row,
            readback_row_stride_bytes: read.layout.copy_row, data,
        });
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_capture_layout_preserves_odd_sizes_and_float_bits() {
        for bytes_per_pixel in [4, 8, 16] {
            for size in [[1, 1], [479, 269], [958, 538], [17, 3]] {
                let layout = Layout::new(size, bytes_per_pixel).unwrap();
                assert_eq!(layout.copy_row % 256, 0);
                assert!(layout.copy_row >= layout.tight_row && layout.copy_row - layout.tight_row < 256);
                let mut padded = vec![0xcdu8; layout.bytes as usize];
                let mut expected = Vec::new();
                for y in 0..size[1] {
                    for x in 0..layout.tight_row {
                        let byte = ((x * 17 + y * 43) & 255) as u8;
                        padded[(y * layout.copy_row + x) as usize] = byte;
                        expected.push(byte);
                    }
                }
                assert_eq!(layout.unpack(size[1], &padded).unwrap(), expected);
            }
        }
        let layout = Layout::new([1, 1], 8).unwrap();
        let mut padded = vec![0; layout.bytes as usize];
        let bits = [0x00, 0x80, 0x01, 0x7e, 0x00, 0xfc, 0xff, 0x7b];
        padded[..8].copy_from_slice(&bits); // -0, NaN, -infinity, largest half.
        assert_eq!(layout.unpack(1, &padded).unwrap(), bits);
    }

    #[test]
    fn raw_capture_rejects_invalid_layout_and_truncated_readback() {
        assert!(Layout::new([0, 10], 8).is_err());
        assert!(Layout::new([10, 10], 3).is_err());
        assert!(Layout::new([u32::MAX, 1], 16).is_err());
        let layout = Layout::new([7, 3], 8).unwrap();
        assert!(layout.unpack(3, &vec![0; layout.bytes as usize - 1]).is_err());
        assert!(layout.unpack(2, &vec![0; layout.bytes as usize]).is_err());
    }
}
