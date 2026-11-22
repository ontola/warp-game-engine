//! Seeded crowd rendering probe. One immutable instance table serves every view.
//! The GPU buckets visible people by projected height and writes indirect draws.
mod geometry;
mod standing;
pub use standing::StandingPose;
mod player_strands;
pub use player_strands::Groom;
pub(crate) use player_strands::PlayerStrands;
pub use geometry::{BodyVertex, hand_frame};

pub const APPEARANCE_ATLAS: &[u8] = include_bytes!("../../../../assets/humans/appearance.png");

use crate::{Camera, Gpu};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Far {
    Cells,
    Mesh,
    Impostor,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Instance {
    pos_yaw: [f32; 4],
    size_phase: [f32; 4],
    color: [f32; 4],
    identity: [f32; 4],
}

fn sample(seed: u64, channel: u64) -> f32 {
    let mut v = seed.wrapping_add(channel.wrapping_mul(0x9e3779b97f4a7c15));
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((v ^ (v >> 31)) >> 40) as f32 / 16777216.
}

// Six independent uniform draws have unit variance after this scaling. Disjoint
// channels make the body reproducible without perturbing appearance or animation.
fn body_deviate(seed: u64, first: u64) -> f32 {
    ((first..first + 6).map(|i| sample(seed, i)).sum::<f32>() - 3.) * std::f32::consts::SQRT_2
}

/// Adult stature sampled about the midpoint of an authored range (ends at 3σ).
pub fn seeded_height(seed: u64, [low, high]: [f32; 2]) -> f32 {
    low + (high - low) * (0.5 + body_deviate(seed, 32).clamp(-3., 3.) / 6.)
}

// Regional breadth offsets in the low twelve bits of each animation channel.
// The high twelve keep the original uniform phase and tempo. The GPU instance
// remains 64 bytes, and the fitted mesh reads the same quantized proportions.
fn shape_phase(phase: f32, bits: u32) -> f32 {
    (((phase * 4096.) as u32) * 4096 + (bits & 4095)) as f32 / 16777216.
}

// A float carries an exact 24-bit RGB value. The instance stays 64 bytes.
fn pack_color(rgb: [f32; 3]) -> f32 {
    let [r,g,b] = rgb.map(|c| (c.clamp(0.,1.)*255.).round() as u32);
    (r | (g<<8) | (b<<16)) as f32
}
fn unpack_color(word: f32) -> [f32; 3] {
    [0,8,16].map(|shift| ((word as u32 >> shift) & 255) as f32 / 255.)
}

// Stored above the nine wardrobe bits: 12-bit sRGB groom and a 3-bit natural
// brow shade. The final 24-bit integer is exactly representable as f32.
#[derive(Clone, Copy)]
struct HairPalette(u32);

impl HairPalette {
    fn select(entropy: u32, skin: Vec3) -> Self {
        let light = ((skin.dot(Vec3::new(0.299, 0.587, 0.114)) - 0.20) / 0.58).clamp(0., 1.);
        let roll = (entropy & 4095) as f32 / 4096.;
        // Art-directed cumulative odds, interpolated across the effective skin tone.
        // Near-black, espresso, chestnut, light brown, honey and pale blond.
        let thresholds = [(0.52, 0.14), (0.87, 0.42), (0.96, 0.66), (0.988, 0.80), (0.997, 0.94)];
        let shade = thresholds.iter().filter(|&&(dark, fair)| roll >= dark + light * (fair-dark)).count();
        let natural = [0x211, 0x321, 0x543, 0x764, 0xa86, 0xcb9];
        let dyed = [0x586, 0x867, 0x659, 0xa65];
        let groom = if (entropy >> 12) & 255 < 6 {
            dyed[((entropy >> 20) & 3) as usize]
        } else { natural[shade] };
        Self(groom | ((shade as u32) << 12))
    }

    fn groom(self) -> Vec3 {
        Vec3::from_array([8, 4, 0].map(|shift| ((self.0 >> shift) & 15u32) as f32 / 15.))
    }

    fn brows(self, skin: Vec3) -> Vec3 {
        let light = (self.0 >> 12) as f32 / 5.;
        Vec3::new(0.09, 0.065, 0.05).lerp(Vec3::new(0.47, 0.36, 0.26), light).lerp(skin, 0.20)
    }
}

#[derive(Default)]
pub struct Constraints {
    pub height_m: Option<f32>,
    pub shirt: Option<[f32; 3]>,
}
impl Instance {
    pub fn hair_style(&self) -> u32 { (self.identity[2] * 3.) as u32 }
    pub fn face_tile(&self) -> usize { (self.identity[3] * 3.) as usize }
    pub fn shirt_color(&self) -> [f32; 3] { unpack_color(self.color[0]) }
    pub fn trouser_color(&self) -> [f32; 3] { unpack_color(self.color[1]) }
    pub fn shoe_color(&self) -> [f32; 3] { unpack_color(self.color[2]) }
    pub fn outfit(&self) -> usize { (self.color[3] as usize).saturating_sub(1) & 3 }
    pub fn chest_print(&self) -> usize { ((self.color[3] as usize & 511).saturating_sub(1)) >> 2 }
    /// Stable timing channel for the fitted player's lids and gaze.
    pub fn face_phase(&self) -> f32 { self.identity[0] }

    pub fn hair_color(&self) -> Vec3 {
        HairPalette((self.color[3] as u32) >> 9).groom()
    }
    fn hair_palette(&self, skin: Vec3) -> HairPalette {
        // Low bits supply independent draws without tying blondness to a face tile.
        HairPalette::select((self.identity[0] * 16777216.) as u32, skin)
    }
    /// Groom and brow tints in sRGB, including a profile's explicit skin colour.
    pub fn hair_and_brows(&self, skin: Vec3) -> (Vec3, Vec3) {
        let palette = self.hair_palette(skin);
        (palette.groom(), palette.brows(skin))
    }
    pub fn skin_color(&self) -> Vec3 {
        let cheeks = [Vec3::new(0.855, 0.659, 0.541), Vec3::new(0.894, 0.741, 0.639), Vec3::new(0.353, 0.224, 0.149)];
        let pigment = Vec3::new(0.28, 0.125, 0.076).lerp(Vec3::new(0.85, 0.55, 0.409), self.identity[1]);
        cheeks[self.face_tile()].lerp(pigment, 0.46)
    }
    /// Metre scale of the normalized human mesh in its local axes.
    pub fn body_scale(&self) -> Vec3 {
        Vec3::new(self.size_phase[0], self.size_phase[1], self.size_phase[0])
    }
    pub fn person(seed: u64, position: Vec3, yaw: f32, rules: &Constraints) -> Self {
        let height = rules
            .height_m
            .unwrap_or_else(|| seeded_height(seed, [1.53, 2.00]));
        let frame = body_deviate(seed, 38).clamp(-3., 3.);
        let fullness = body_deviate(seed, 44).clamp(-3., 3.);
        let taper = body_deviate(seed, 50).clamp(-3., 3.);
        // A broader frame carries the whole body. Fullness especially fills
        // the waist, while taper redistributes some breadth from hips to chest.
        let breadth = 1. + 0.040 * frame + 0.018 * fullness
            - 0.20 * ((height - 1.75) / 1.75).clamp(-0.25, 0.25);
        let regions = [0.014 * fullness + 0.024 * taper,
                       0.042 * fullness - 0.005 * taper,
                       0.025 * fullness - 0.019 * taper];
        let mut packed = 0;
        for (i, offset) in regions.into_iter().enumerate() {
            let byte = (128. + offset.clamp(-0.15, 0.15) * (127. / 0.15)).round() as u32;
            packed |= byte << (i * 8);
        }
        let palette = [
            [0.65, 0.21, 0.16],
            [0.20, 0.36, 0.56],
            [0.19, 0.40, 0.32],
            [0.75, 0.63, 0.36],
            [0.49, 0.35, 0.48],
            [0.73, 0.72, 0.68],
            [0.21, 0.23, 0.26],
            [0.46, 0.52, 0.60],
        ];
        let shirt = rules
            .shirt
            .unwrap_or(palette[(sample(seed, 2) * 8.) as usize]);
        let trousers = [[0.20,0.29,0.39],[0.42,0.55,0.64],[0.23,0.24,0.25],[0.53,0.49,0.36],
            [0.34,0.40,0.28],[0.38,0.27,0.21],[0.29,0.34,0.39],[0.64,0.61,0.53]];
        let shoes = [[0.17,0.17,0.19],[0.72,0.71,0.66],[0.41,0.29,0.20],[0.27,0.34,0.42],
            [0.47,0.22,0.19],[0.35,0.37,0.32]];
        let outfit = (sample(seed,10)*3.) as u32;
        let print = if sample(seed, 13) < 0.58 {
            0
        } else if sample(seed, 15) < 0.5 {
            1 + (sample(seed, 14) * 100.) as u32
        } else {
            101 + (sample(seed, 14) * 6.) as u32
        };
        let mut person = Self {
            pos_yaw: position.extend(yaw).into(),
            size_phase: [
                height * breadth,
                height,
                shape_phase(sample(seed, 4), packed),
                shape_phase(sample(seed, 5), packed >> 12),
            ],
            color: [pack_color(shirt), pack_color(trousers[(sample(seed,11)*8.) as usize]),
                pack_color(shoes[(sample(seed,12)*6.) as usize]), (1+outfit+4*print) as f32],
            identity: std::array::from_fn(|i| sample(seed, 6 + i as u64)),
        };
        let palette = person.hair_palette(person.skin_color());
        person.color[3] = ((person.color[3] as u32) | (palette.0 << 9)) as f32;
        person
    }
    pub fn spectator(seed: u64, seat_index: usize, seat: &crate::crowd::Seat) -> Self {
        Self::person(
            seed.wrapping_mul(0x9e3779b97f4a7c15) ^ seat_index as u64,
            seat.pos,
            seat.yaw,
            &Constraints {
                height_m: Some(seat.scale),
                shirt: None,
            },
        )
    }

    pub fn stand(pos: Vec3, yaw: f32, size: Vec3, color: [f32; 3]) -> Self {
        Self {
            pos_yaw: pos.extend(yaw).into(),
            size_phase: size.extend(0.).into(),
            color: [pack_color(color), 0., 0., 0.],
            identity: [0.; 4],
        }
    }
}
struct Geometry {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    count: u32,
}
impl Geometry {
    fn upload(gpu: &Gpu, m: geometry::Mesh) -> Self {
        let buf = |data, usage| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("human geometry"),
                    contents: data,
                    usage,
                })
        };
        let vertices:Vec<_> = m.vertices.iter().map(geometry::Vertex::packed).collect();
        let indices:Vec<_> = m.indices.iter().map(|&i| u16::try_from(i).expect("crowd mesh exceeds 16-bit indices")).collect();
        Self {
            vertex: buf(
                bytemuck::cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            index: buf(bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX),
            count: m.indices.len() as u32,
        }
    }
    fn bytes(&self) -> u64 {
        self.vertex.size() + self.index.size()
    }
}

// Allocate the shared appearance atlas on first close demand.
fn appearance(
    gpu: &Gpu,
    pipelines: [&wgpu::RenderPipeline; 2],
    image: &image::RgbaImage,
) -> [wgpu::BindGroup; 2] {
    let extent = wgpu::Extent3d {
        width: image.width(),
        height: image.height(),
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("human appearance"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        image.as_raw(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(extent.width * 4),
            rows_per_image: None,
        },
        extent,
    );
    let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    pipelines.map(|render| gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("human appearance"),
        layout: &render.get_bind_group_layout(1),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    }))
}

const VISIBLE_SEAT_BYTES: u64 = 48;
const HUMAN_SHADER: &str = concat!(
    include_str!("../shaders/crowd_jump.wgsl"), "\n", include_str!("../shaders/crowd_displacement.wgsl"), "\n",
    include_str!("../shaders/surface_fog.wgsl"), "\n",
    include_str!("../shaders/incident_lights.wgsl"), "\n",
    include_str!("strands.wgsl"), "\n",
    include_str!("humans.wgsl"),
);

pub struct ReviewRenderer {
    compute: wgpu::ComputePipeline,
    render: wgpu::RenderPipeline,
    selection: wgpu::BindGroup,
    light_groups: crate::frame::resource_cache::BindGroups<8>,
    fog_groups: crate::frame::resource_cache::BindGroups<8>,
    fog_layout: Option<wgpu::BindGroupLayout>,
    light_layout: Option<wgpu::BindGroupLayout>,
    drawing: wgpu::BindGroup,
    appearance: [wgpu::BindGroup; 2],
    strands: wgpu::RenderPipeline,
    strand_group: wgpu::BindGroup,
    strand_fog_groups: crate::frame::resource_cache::BindGroups<8>,
    strand_fog_layout: Option<wgpu::BindGroupLayout>,
    camera: wgpu::Buffer,
    args: wgpu::Buffer,
    read: wgpu::Buffer,
    timer: Option<(wgpu::QuerySet, wgpu::Buffer)>,
    meshes: Vec<Geometry>,
    count: u32,
    far: Far,
    first_pose: Option<u32>,
    seat_bounds: Vec<(Vec3, Vec3)>,
    pending: Option<std::sync::mpsc::Receiver<(geometry::Mesh, image::RgbaImage)>>,
    pub appearance_bytes: u64,
    pub detail_ready: bool,
    pub resident_buffer_bytes: u64,
}
pub struct ViewSample {
    pub counts: [u32; 3],
    pub gpu_ms: Option<f64>,
}
impl ReviewRenderer {
    pub fn new(gpu: &Gpu, actors: &[Instance], far: Far) -> Result<Self, String> {
        Self::create(gpu, actors, far, None)
    }

    pub(crate) fn for_match(
        gpu: &Gpu,
        actors: &[Instance],
        poses: &wgpu::Buffer,
        first: u32,
    ) -> Result<Self, String> {
        Self::create(gpu, actors, Far::Impostor, Some((poses, first)))
    }

    fn create(
        gpu: &Gpu,
        actors: &[Instance],
        far: Far,
        game: Option<(&wgpu::Buffer, u32)>,
    ) -> Result<Self, String> {
        // Rows of neighbouring seats keep first-detail demand cheap to check.
        let seat_bounds = if game.is_some() {
            actors
                .chunks(32)
                .map(|row| {
                    row.iter().fold(
                        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                        |(lo, hi), a| {
                            let p = Vec3::from_slice(&a.pos_yaw);
                            (lo.min(p), hi.max(p + Vec3::Y * a.size_phase[1]))
                        },
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let count = u32::try_from(actors.len()).map_err(|_| "too many instances")?;
        if count == 0
            || u64::from(count) * 5 * VISIBLE_SEAT_BYTES > u64::from(gpu.limits.max_storage_buffer_binding_size)
        {
            return Err("invalid human instance count".into());
        }
        use wgpu::BufferUsages as U;
        let buffer = |size, usage| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("human review"),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let camera = buffer(224, U::UNIFORM | U::COPY_DST);
        let actors = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("crowd identities"),
                contents: bytemuck::cast_slice(actors),
                usage: U::STORAGE,
            });
        let dummy = game.is_none().then(|| buffer(80, U::STORAGE));
        let poses = game.map(|(b, _)| b).or(dummy.as_ref()).unwrap();
        let ids = buffer(u64::from(count) * 5 * VISIBLE_SEAT_BYTES, U::STORAGE);
        let args = buffer(100, U::STORAGE | U::COPY_DST | U::COPY_SRC | U::INDIRECT);
        let read = buffer(120, U::MAP_READ | U::COPY_DST);
        let timer = gpu
            .features
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
            .then(|| {
                (
                    gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: None,
                        ty: wgpu::QueryType::Timestamp,
                        count: 2,
                    }),
                    buffer(16, U::QUERY_RESOLVE | U::COPY_SRC),
                )
            });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("warp humans"),
                source: wgpu::ShaderSource::Wgsl(HUMAN_SHADER.into()),
            });
        let compute = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &shader,
                entry_point: Some(if game.is_some() { "select_match" } else { "select_lod" }),
                compilation_options: Default::default(),
                cache: None,
            });
        let attrs = wgpu::vertex_attr_array![0=>Snorm16x4,1=>Snorm8x4,2=>Snorm16x4,3=>Uint8x4,4=>Unorm8x4,5=>Snorm8x4,6=>Snorm8x4,7=>Snorm8x4,8=>Snorm8x4];
        let pipeline = |entry, buffers: &[Option<wgpu::VertexBufferLayout>]| gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("warp humans"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(if game.is_some() {
                        wgpu::CompareFunction::Greater
                    } else {
                        wgpu::CompareFunction::Less
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if game.is_some() { "fs_match" } else { "fs_review" }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: if game.is_some() {
                            wgpu::TextureFormat::Rgba16Float
                        } else {
                            wgpu::TextureFormat::Rgba8Unorm
                        },
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let render = pipeline("vs",&[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<geometry::PackedVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })]);
        let strands = pipeline("crowd_strand",&[]);
        fn entry(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            }
        }
        let selection = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &compute.get_bind_group_layout(0),
            entries: &[
                entry(0, &camera),
                entry(1, &actors),
                entry(2, &ids),
                entry(3, &args),
                entry(5, poses),
            ],
        });
        let drawing = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &render.get_bind_group_layout(0),
            entries: &[
                entry(0, &camera),
                entry(1, &actors),
                entry(4, &ids),
                entry(5, poses),
            ],
        });
        let guides = player_strands::guide_buffer(gpu);
        let strand_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crowd strand sources"),
            layout: &strands.get_bind_group_layout(0),
            entries: &[entry(0,&camera),entry(1,&actors),entry(4,&ids),entry(5,poses),entry(8,&guides)],
        });
        let meshes: Vec<_> = [
            geometry::stand(),
            geometry::person(1, far, false),
            geometry::person(2, far, false),
            geometry::person(4, far, false),
        ]
        .into_iter()
        .map(|m| Geometry::upload(gpu, m))
        .collect();
        let resident_buffer_bytes = meshes.iter().map(Geometry::bytes).sum::<u64>()
            + guides.size()
            + actors.size()
            + ids.size()
            + args.size()
            + camera.size()
            + read.size()
            + timer.as_ref().map_or(0, |t| t.1.size());
        let appearance = appearance(
            gpu,
            [&render,&strands],
            &image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4])),
        );
        Ok(Self {
            strand_fog_layout: game.map(|_| strands.get_bind_group_layout(2)),
            strand_fog_groups:Default::default(),
            strands, strand_group, appearance,
            light_layout: game.map(|_| compute.get_bind_group_layout(1)),
            light_groups: Default::default(),
            fog_layout: game.map(|_| render.get_bind_group_layout(2)),
            fog_groups: Default::default(),
            compute,
            render,
            selection,
            drawing,
            camera,
            args,
            read,
            timer,
            meshes,
            count,
            far,
            first_pose: game.map(|(_, first)| first),
            seat_bounds,
            pending: None,
            appearance_bytes: 4,
            detail_ready: false,
            resident_buffer_bytes,
        })
    }

    /// Decode the shared CPU page while intermediate detail is visible.
    pub fn prepare_detail(&mut self) {
        if self.detail_ready || self.pending.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending = Some(rx);
        let far = self.far;
        let prepare = move || {
            let mesh = geometry::person(4, far, true);
            let image =
                image::load_from_memory(APPEARANCE_ATLAS)
                    .expect("cooked human atlas")
                    .into_rgba8();
            let _ = tx.send((mesh, image));
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(prepare);
        #[cfg(target_arch = "wasm32")]
        prepare();
    }

    /// Keep the fallback until both geometry and surface data are ready.
    pub fn load_detail(&mut self, gpu: &Gpu) {
        if self.detail_ready {
            return;
        }
        self.prepare_detail();
        let Ok((mesh, image)) = self.pending.as_ref().unwrap().try_recv() else {
            return;
        };
        self.pending = None;
        let head = Geometry::upload(gpu, mesh);
        self.resident_buffer_bytes =
            self.resident_buffer_bytes - self.meshes[3].bytes() + head.bytes();
        self.meshes[3] = head;
        self.appearance_bytes = image.as_raw().len() as u64;
        self.appearance = appearance(gpu, [&self.render,&self.strands], &image);
        self.detail_ready = true;
    }
    // Queue writes to one uniform would make every split-screen draw use the last camera.
    // Encoder copies keep the uniform and indirect counters ordered with each view.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_match(
        &mut self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        cam: &Camera,
        vp: Mat4,
        view: Mat4,
        focal: f32,
        light: &crate::Lighting,
        shadow_vp: &[[f32; 4]; 4],
        lights: &[(u32, wgpu::BindingResource)],
        atmosphere: &[(u32, wgpu::BindingResource)],
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        broadcast: Option<&std::ops::Range<u32>>,
    ) {
        if !self.detail_ready {
            let nearest = self
                .seat_bounds
                .iter()
                .map(|(lo, hi)| cam.pos.distance_squared(cam.pos.clamp(*lo, *hi)))
                .fold(f32::INFINITY, f32::min);
            if nearest < (2.2 * focal / 32. + 8.).powi(2) {
                self.prepare_detail();
            }
            if nearest < (2.2 * focal / 170. + 8.).powi(2) {
                self.load_detail(gpu);
            }
        }
        let mut words = [0u32; 81];
        let u: &mut [f32] = bytemuck::cast_slice_mut(&mut words[..56]);
        u[..16].copy_from_slice(&vp.to_cols_array());
        let axes = view.transpose();
        u[16..20].copy_from_slice(&axes.x_axis.to_array());
        u[20..24].copy_from_slice(&axes.y_axis.to_array());
        u[24..28].copy_from_slice(&[
            1.,
            self.first_pose.unwrap() as f32,
            light.sky_lux,
            light.sun_lux,
        ]);
        u[28..31].copy_from_slice(&light.sun_dir.to_array());
        u[31] = light.shadow_strength.clamp(0., 1.);
        let seats = broadcast.cloned().unwrap_or(0..self.count);
        if broadcast.is_some() {
            // The TV has its own short lens-to-subject distance and SDR exposure.
            u[24] = 2.;
            u[19] = crate::auto_exposure::exposure(light.ev100);
            u[23] = seats.end as f32;
            u[35] = seats.start as f32;
        }
        u[40..56].copy_from_slice(bytemuck::cast_slice(shadow_vp));
        u[32..35].copy_from_slice(&light.sun_color.to_array());
        u[36..40].copy_from_slice(&[light.time, focal, self.far as u32 as f32, self.count as f32]);
        for (mesh, bin) in self.meshes.iter().zip([0, 1, 2, 4]) {
            words[56 + 5 * bin] = mesh.count;
        }
        words[56+15] = if self.detail_ready && broadcast.is_none() { 1536*54 } else { 0 };
        let upload = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("human camera and indirect reset"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        enc.copy_buffer_to_buffer(&upload, 0, &self.camera, 0, 224);
        enc.copy_buffer_to_buffer(&upload, 224, &self.args, 0, 100);
        let lamps = self.light_groups.get(gpu, self.light_layout.as_ref().unwrap(), "human scene lights", lights);
        let fog = self.fog_groups.get(gpu, self.fog_layout.as_ref().unwrap(), "human scene fog", atmosphere);
        let hair_fog=self.strand_fog_groups.get(gpu,self.strand_fog_layout.as_ref().unwrap(),"crowd strand atmosphere",atmosphere);
        self.encode_passes(enc, color, depth, seats.len() as u32, Some((&lamps, &fog, &hair_fog)));
    }

    fn encode_passes(
        &self,
        enc: &mut wgpu::CommandEncoder,
        cv: &wgpu::TextureView,
        dv: &wgpu::TextureView,
        seat_count: u32,
        match_groups: Option<(&wgpu::BindGroup, &wgpu::BindGroup, &wgpu::BindGroup)>,
    ) {
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Warp Humans detail selection"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &self.selection, &[]);
            if let Some((lights, _, _)) = match_groups {
                pass.set_bind_group(1, lights, &[]);
            }
            pass.dispatch_workgroups(seat_count.div_ceil(64), 1, 1);
        }
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Warp Humans"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: cv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if self.first_pose.is_some() {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.66,
                                g: 0.77,
                                b: 0.86,
                                a: 1.,
                            })
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: dv,
                    depth_ops: Some(wgpu::Operations {
                        load: if self.first_pose.is_some() {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(1.)
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.render);
            pass.set_bind_group(0, &self.drawing, &[]);
            pass.set_bind_group(1, &self.appearance[0], &[]);
            if let Some((_, fog, _)) = match_groups {
                pass.set_bind_group(2, fog, &[]);
            }
            for (mesh, bin) in self.meshes.iter().zip([0, 1, 2, 4]) {
                pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                pass.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed_indirect(&self.args, bin * 20);
            }
            if self.detail_ready {
                pass.set_pipeline(&self.strands);
                pass.set_bind_group(0,&self.strand_group,&[]);
                pass.set_bind_group(1,&self.appearance[1],&[]);
                if let Some((_,_,fog))=match_groups {pass.set_bind_group(2,fog,&[]);}
                pass.draw_indirect(&self.args,15*4);
            }
        }
    }
    pub fn draw(
        &self,
        gpu: &Gpu,
        cam: &Camera,
        time: f32,
        color: &wgpu::Texture,
        depth: &wgpu::Texture,
    ) -> Result<ViewSample, String> {
        let (w, h) = (color.width(), color.height());
        let look = (cam.target - cam.pos).normalize();
        let right = look.cross(Vec3::Y).normalize();
        let up = right.cross(look);
        let vp = Mat4::perspective_rh(cam.fov_y, w as f32 / h as f32, cam.near, 2000.)
            * Mat4::look_at_rh(cam.pos, cam.target, Vec3::Y);
        let mut u = [0f32; 40];
        u[..16].copy_from_slice(&vp.to_cols_array());
        u[16..19].copy_from_slice(&right.to_array());
        u[20..23].copy_from_slice(&up.to_array());
        u[36..].copy_from_slice(&[
            time,
            h as f32 / (2. * (cam.fov_y * 0.5).tan()),
            self.far as u32 as f32,
            self.count as f32,
        ]);
        gpu.queue
            .write_buffer(&self.camera, 0, bytemuck::cast_slice(&u));
        let mut commands = [0u32; 25];
        for (mesh, bin) in self.meshes.iter().zip([0, 1, 2, 4]) {
            commands[bin * 5] = mesh.count;
        }
        commands[15]=if self.detail_ready {1536*54} else {0};
        gpu.queue
            .write_buffer(&self.args, 0, bytemuck::cast_slice(&commands));
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        if let Some((q, _)) = &self.timer {
            enc.write_timestamp(q, 0);
        }
        self.encode_passes(
            &mut enc,
            &color.create_view(&Default::default()),
            &depth.create_view(&Default::default()),
            self.count,
            None,
        );
        if let Some((q, b)) = &self.timer {
            enc.write_timestamp(q, 1);
            enc.resolve_query_set(q, 0..2, b, 0);
            enc.copy_buffer_to_buffer(b, 0, &self.read, 104, 16);
        }
        enc.copy_buffer_to_buffer(&self.args, 0, &self.read, 0, 100);
        gpu.queue.submit([enc.finish()]);
        self.read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.wait();
        let data = self
            .read
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let counts = [1, 2, 4].map(|bin| {
            let i = bin * 20 + 4;
            u32::from_le_bytes(data[i..i + 4].try_into().unwrap())
        });
        let gpu_ms = self.timer.as_ref().map(|_| {
            let ts = |i| u64::from_le_bytes(data[i..i + 8].try_into().unwrap());
            ts(112).wrapping_sub(ts(104)) as f64 * f64::from(gpu.queue.get_timestamp_period()) / 1e6
        });
        drop(data);
        self.read.unmap();
        Ok(ViewSample { counts, gpu_ms })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_fragments_receive_scene_fog_on_every_detail_level() {
        let module = naga::front::wgsl::parse_str(HUMAN_SHADER)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(HUMAN_SHADER)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all(),
        ).validate(&module).unwrap();
        let seat = module.types.iter().find(|(_, t)| t.name.as_deref() == Some("SeatLight")).unwrap().1;
        let naga::TypeInner::Struct { span, .. } = seat.inner else { panic!("expected visible spectator record"); };
        assert_eq!(u64::from(span), VISIBLE_SEAT_BYTES);
        for (name, expected) in [("fs_match", true), ("fs_review", false)] {
            let i = module.entry_points.iter().position(|e| e.name == name).unwrap();
            let stage = info.get_entry_point(i);
            for (handle, var) in module.global_variables.iter() {
                if var.binding.as_ref().is_some_and(|b| b.group == 2) {
                    assert_eq!(!stage[handle].is_empty(), expected, "{name} {:?}", var.name);
                }
            }
        }
    }

    #[test]
    fn fair_hair_odds_and_dye_are_seeded_independently() {
        let mut blond = [0; 2];
        let mut dyed = 0;
        for seed in 0..8192 {
            let p = Instance::person(seed, Vec3::ZERO, 0., &Constraints::default());
            let packed = HairPalette((p.color[3] as u32) >> 9);
            assert_eq!(packed.0, p.hair_palette(p.skin_color()).0);
            assert_eq!(std::mem::size_of::<Instance>(), 64);
            assert!(p.outfit() < 3 && p.chest_print() <= 106);
            let pair = [Vec3::new(0.28, 0.15, 0.09), Vec3::new(0.95, 0.80, 0.70)]
                .map(|skin| p.hair_palette(skin));
            // The natural shade can only get lighter for a fixed roll.
            assert!(pair[0].0 >> 12 <= pair[1].0 >> 12);
            let is_dye = |h: HairPalette| matches!(h.0 & 0xfff, 0x586 | 0x867 | 0x659 | 0xa65);
            assert_eq!(is_dye(pair[0]), is_dye(pair[1]));
            if is_dye(pair[0]) {
                assert_eq!(pair[0].groom(), pair[1].groom());
                dyed += 1;
            } else {
                for (n, h) in blond.iter_mut().zip(pair) {
                    *n += usize::from(h.0 >> 12 >= 4);
                }
            }
        }
        assert!(blond[0] > 0 && blond[1] > blond[0] * 8);
        assert!((100..300).contains(&dyed));
        println!("8192 seeds: light hair {blond:?}; dyed {dyed}");
    }

    #[test]
    fn overrides_preserve_other_identity_channels() {
        let seed = 52;
        let raw = Instance::person(seed, Vec3::ZERO, 0.0, &Constraints::default());
        let same = Instance::person(seed, Vec3::ZERO, 0.0, &Constraints::default());
        assert_eq!(bytemuck::bytes_of(&raw), bytemuck::bytes_of(&same));
        let red = Instance::person(
            seed,
            Vec3::ZERO,
            0.0,
            &Constraints {
                height_m: Some(1.76),
                shirt: Some([0.8, 0.1, 0.2]),
            },
        );
        assert_eq!(raw.identity, red.identity);
        assert_eq!(raw.size_phase[2..], red.size_phase[2..]);
        assert_eq!(red.size_phase[1], 1.76);
        assert!((Vec3::from(red.shirt_color())-Vec3::new(0.8,0.1,0.2)).abs().max_element() <= 0.5/255.);
        assert_ne!(
            raw.identity,
            Instance::person(seed + 1, Vec3::ZERO, 0.0, &Constraints::default()).identity
        );
    }
}
