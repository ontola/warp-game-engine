//! Native-resolution HUD geometry and cached text, shared by every split view.
//! Coordinates and clips are physical pixels; all rectangles are x/y/width/height.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glyphon::{Attrs, Buffer, Cache, Color, ColorMode, ContentType, CustomGlyph, CustomGlyphId, RasterizedCustomGlyph, Family, FontSystem, Metrics, Resolution, Shaping, Style, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap};
use glyphon::cosmic_text::Align;
use std::{collections::{HashMap, HashSet}, ops::Range, time::Instant};
use wgpu::util::DeviceExt;

const BODY_FONT: &[u8] = include_bytes!("../../../assets/fonts/BarlowCondensed-SemiBold.ttf");
const TITLE_FONT: &[u8] = include_bytes!("../../../assets/fonts/BarlowCondensed-ExtraBoldItalic.ttf");
/// Barlow is packaged under SIL OFL 1.1; keep this notice with distributions.
pub const FONT_LICENSE: &str = include_str!("../../../assets/fonts/OFL-BarlowCondensed.txt");
const WARM_TEXT: &str = "0123456789:-+. /()<> ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz";
const MAX_RUNS: usize = 2048;
const MAX_VERTICES: usize = 131_072;
const MAX_LAYERS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFace {
    Body,
    Title,
    /// A face a game registered with [`register_font`].
    Custom(u16),
}

/// Fonts games add next to the two bundled faces, in registration order.
static REGISTERED: std::sync::Mutex<Vec<&'static [u8]>> = std::sync::Mutex::new(Vec::new());

/// Make a font file (TTF/OTF bytes) available to every HUD renderer and to
/// [`measure`]. Registering the same bytes again returns the same face.
pub fn register_font(data: &'static [u8]) -> FontFace {
    let mut fonts = REGISTERED.lock().unwrap_or_else(|e| e.into_inner());
    let k = fonts.iter().position(|f| std::ptr::eq(f.as_ptr(), data.as_ptr()) && f.len() == data.len()).unwrap_or_else(|| { fonts.push(data); fonts.len() - 1 });
    FontFace::Custom(k as u16)
}

fn registered() -> Vec<&'static [u8]> { REGISTERED.lock().unwrap_or_else(|e| e.into_inner()).clone() }

/// Width and height in pixels of `text` laid out on one line (height is
/// lines times the 1.2 line height every run uses), for fitting and
/// centring text before drawing it.
pub fn measure(text: &str, face: FontFace, size: f32) -> [f32; 2] {
    static CACHE: std::sync::Mutex<Option<FontCache>> = std::sync::Mutex::new(None);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() { *guard = FontCache::new().ok(); }
    let Some(cache) = guard.as_mut() else { return [0.0; 2] };
    cache.sync_registered();
    if !size.is_finite() || size <= 0.0 { return [0.0; 2]; }
    let buffer = cache.buffer(text, face, size, 1.0e6, TextAlign::Left, Wrap::None);
    let (mut w, mut lines) = (0.0f32, 0usize);
    for run in buffer.layout_runs() { w = w.max(run.line_w); lines += 1; }
    [w, lines.max(1) as f32 * size * 1.2]
}
/// Distance from a run's `position` (layout top) down to the first line's
/// baseline, for callers that place text by its baseline (as Godot does).
pub fn baseline(face: FontFace, size: f32) -> f32 {
    static CACHE: std::sync::Mutex<Option<FontCache>> = std::sync::Mutex::new(None);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() { *guard = FontCache::new().ok(); }
    let Some(cache) = guard.as_mut() else { return size };
    cache.sync_registered();
    if !size.is_finite() || size <= 0.0 { return 0.0; }
    let buffer = cache.buffer("Hg", face, size, 1.0e6, TextAlign::Left, Wrap::None);
    buffer.layout_runs().next().map_or(size, |run| run.line_y)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextAlign { Left, Center, Right }

#[derive(Clone, Debug)]
pub struct TextRun {
    pub id: u64,
    pub text: String,
    /// Top-left of the text layout, not the baseline.
    pub position: [f32; 2],
    pub bounds: [f32; 4],
    pub font_size: f32,
    /// Straight-alpha sRGB bytes.
    pub color: [u8; 4],
    pub font: FontFace,
    pub align: TextAlign,
    pub width: f32,
}

/// A stroke around a text run ([`HudDrawList::outline`]): `width` pixels
/// outside the letters, in straight-alpha sRGB `color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline { pub width: f32, pub color: [u8; 4] }

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct HudVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub clip: [f32; 4],
    // 0: plain triangle; 1: circle (center x/y, radius); 2: line (distance, half-width);
    // 3: picture (uv in x/y, the image bound by its run in `HudDrawList`).
    pub shape: [f32; 4],
}

/// A picture uploaded with `HudRenderer::set_image`, named by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HudImage(pub u64);

/// Pixel art wants `Nearest`; photos and covers `Linear` (mipmapped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFilter { Linear, Nearest }

#[derive(Clone, Debug)]
pub struct HudDrawList {
    pub size: [u32; 2],
    pub vertices: Vec<HudVertex>,
    pub text: Vec<TextRun>,
    overlay_boundaries: Vec<(usize, usize)>,
    /// (first vertex, picture) where the bound picture changes; plain shapes use None.
    image_runs: Vec<(u32, Option<HudImage>)>,
    /// Pictures this list brings along ([`HudDrawList::provide_image`]):
    /// whoever prepares it uploads the ones it does not hold yet.
    pub pictures: Vec<(HudImage, std::sync::Arc<crate::picture::Picture>, ImageFilter)>,
    /// Text runs drawn with a stroke, by run id.
    pub outlines: HashMap<u64, Outline>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HudLayer { vertices: Range<u32>, text: Range<usize> }

impl HudDrawList {
    pub fn new(size: [u32; 2]) -> Self { Self { size, vertices: Vec::new(), text: Vec::new(), overlay_boundaries: Vec::new(), image_runs: Vec::new(), pictures: Vec::new(), outlines: HashMap::new() } }

    /// Draw text run `id` with a real stroke of `width` pixels around its
    /// letters (under the fill), for text over busy scenes.
    pub fn outline(&mut self, id: u64, width: f32, color: [u8; 4]) {
        if width > 0.0 && width.is_finite() { self.outlines.insert(id, Outline { width: width.min(64.0), color }); }
    }

    fn bind(&mut self, image: Option<HudImage>) {
        let current = self.image_runs.last().and_then(|r| r.1);
        if self.image_runs.is_empty() && image.is_none() { return; }
        if current != image || self.image_runs.is_empty() { self.image_runs.push((self.vertices.len() as u32, image)); }
    }

    /// Vertex ranges of `range` with their pictures, in draw order.
    fn image_spans(&self, range: Range<u32>) -> Vec<(Range<u32>, Option<HudImage>)> {
        let mut spans = vec![];
        let mut start = range.start;
        let mut image = self.image_runs.iter().take_while(|r| r.0 <= start).last().and_then(|r| r.1);
        for &(at, next) in self.image_runs.iter().filter(|r| r.0 > range.start && r.0 < range.end) {
            if at > start { spans.push((start..at, image)); }
            start = at;
            image = next;
        }
        if range.end > start { spans.push((start..range.end, image)); }
        spans
    }

    /// Bring a picture along for [`HudDrawList::image`]: the renderer that
    /// prepares this list uploads it once (again only when a different
    /// `Arc` arrives under the same id). Lets a game whose shell owns the
    /// HUD renderer use pictures without reaching it.
    pub fn provide_image(&mut self, id: HudImage, picture: &std::sync::Arc<crate::picture::Picture>, filter: ImageFilter) {
        if !self.pictures.iter().any(|p| p.0 == id) { self.pictures.push((id, picture.clone(), filter)); }
    }

    /// Draw `image` into `rect`, sampling `uv` (x/y/width/height in 0..1; a
    /// negative width or height mirrors), times `tint` (straight-alpha sRGB).
    pub fn image(&mut self, rect: [f32; 4], image: HudImage, uv: [f32; 4], tint: [f32; 4], clip: [f32; 4]) {
        let [x, y, w, h] = rect;
        if w <= 0.0 || h <= 0.0 { return; }
        self.image_quad([[x, y], [x + w, y], [x + w, y + h], [x, y + h]], image, uv, tint, clip);
    }

    /// Any four corners (clockwise from the top-left of the picture), e.g. rotated.
    pub fn image_quad(&mut self, points: [[f32; 2]; 4], image: HudImage, uv: [f32; 4], tint: [f32; 4], clip: [f32; 4]) {
        let [u, v, uw, vh] = uv;
        let uvs = [[u, v], [u + uw, v], [u + uw, v + vh], [u, v + vh]];
        self.bind(Some(image));
        for i in [0, 1, 2, 0, 2, 3] { self.vertices.push(HudVertex { position: points[i], color: tint, clip, shape: [uvs[i][0], uvs[i][1], 0.0, 3.0] }); }
    }

    /// Start a layer above ALL geometry and text already in the list. Within
    /// each layer, geometry is drawn before text, as in the original HUD API.
    /// Repeated calls without intervening content do not create empty layers.
    /// Keep appending to the public vectors; truncating prior layers is invalid.
    pub fn begin_overlay(&mut self) {
        let end = (self.vertices.len(), self.text.len());
        if end != self.overlay_boundaries.last().copied().unwrap_or((0, 0)) {
            self.overlay_boundaries.push(end);
        }
    }

    fn layer_ranges(&self) -> Result<Vec<HudLayer>, String> {
        if self.overlay_boundaries.len() > MAX_LAYERS {
            return Err("HUD draw list exceeds layer capacity".into());
        }
        let mut result = Vec::new();
        let mut start = (0, 0);
        for end in self.overlay_boundaries.iter().copied()
            .chain(std::iter::once((self.vertices.len(), self.text.len())))
        {
            if end.0 < start.0 || end.1 < start.1
                || end.0 > self.vertices.len() || end.1 > self.text.len()
                || end.0 % 3 != 0 || end.0 > MAX_VERTICES
            {
                return Err("HUD overlay boundary no longer matches complete geometry/text".into());
            }
            if end != start {
                result.push(HudLayer { vertices: start.0 as u32..end.0 as u32, text: start.1..end.1 });
            }
            start = end;
        }
        if result.len() > MAX_LAYERS { return Err("HUD draw list exceeds layer capacity".into()); }
        Ok(result)
    }

    fn quad(&mut self, points: [[f32; 2]; 4], shapes: [[f32; 4]; 4], color: [f32; 4], clip: [f32; 4]) {
        self.bind(None);
        for i in [0, 1, 2, 0, 2, 3] { self.vertices.push(HudVertex { position: points[i], color, clip, shape: shapes[i] }); }
    }

    pub fn rect(&mut self, rect: [f32; 4], color: [f32; 4], clip: [f32; 4]) {
        let [x,y,w,h]=rect;
        if w<=0.0 || h<=0.0 {return;}
        self.quad([[x,y],[x+w,y],[x+w,y+h],[x,y+h]], [[0.0;4];4], color, clip);
    }

    pub fn triangle(&mut self, points: [[f32; 2]; 3], color: [f32; 4], clip: [f32; 4]) {
        self.bind(None);
        for position in points {self.vertices.push(HudVertex{position,color,clip,shape:[0.0;4]});}
    }

    pub fn line(&mut self, from: [f32; 2], to: [f32; 2], width: f32, color: [f32; 4], clip: [f32; 4]) {
        let dx=to[0]-from[0];let dy=to[1]-from[1];let length=dx.hypot(dy);
        if length<=0.0001 || width<=0.0 {return;}
        let half=width*0.5;let extent=half+1.0;
        let nx=-dy/length*extent;let ny=dx/length*extent;
        self.quad([[from[0]-nx,from[1]-ny],[to[0]-nx,to[1]-ny],[to[0]+nx,to[1]+ny],[from[0]+nx,from[1]+ny]],
            [[-extent,half,0.0,2.0],[-extent,half,0.0,2.0],[extent,half,0.0,2.0],[extent,half,0.0,2.0]],color,clip);
    }

    /// A filled polygon, fanned from its centroid: right for convex and
    /// star-shaped outlines (plates, hearts, stars). No edge antialiasing.
    pub fn polygon(&mut self, points: &[[f32; 2]], color: [f32; 4], clip: [f32; 4]) {
        if points.len() < 3 { return; }
        let n = points.len() as f32;
        let c = points.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n]);
        for k in 0..points.len() { self.triangle([c, points[k], points[(k + 1) % points.len()]], color, clip); }
    }

    /// A ring segment from angle `from` to `to` (radians, 0 = +x, clockwise
    /// on screen), `width` thick, centred on `radius`.
    pub fn arc(&mut self, center: [f32; 2], radius: f32, from: f32, to: f32, width: f32, color: [f32; 4], clip: [f32; 4]) {
        let steps = (((to - from).abs() * radius.max(1.0) / 6.0).ceil() as usize).clamp(2, 96);
        let (r0, r1) = ((radius - width * 0.5).max(0.0), radius + width * 0.5);
        let at = |a: f32, r: f32| [center[0] + a.cos() * r, center[1] + a.sin() * r];
        for k in 0..steps {
            let (a, b) = (from + (to - from) * k as f32 / steps as f32, from + (to - from) * (k + 1) as f32 / steps as f32);
            let q = [at(a, r0), at(a, r1), at(b, r1), at(b, r0)];
            self.triangle([q[0], q[1], q[2]], color, clip);
            self.triangle([q[0], q[2], q[3]], color, clip);
        }
    }

    pub fn circle(&mut self, center: [f32; 2], radius: f32, color: [f32; 4], clip: [f32; 4]) {
        if radius<=0.0 {return;}
        let [x,y]=center;let r=radius+1.0;
        self.quad([[x-r,y-r],[x+r,y-r],[x+r,y+r],[x-r,y+r]],[[x,y,radius,1.0];4],color,clip);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HudStats {
    pub text_runs: usize,
    /// Number of shaped glyphs in the visible draw list, before pixel clipping.
    pub glyphs: usize,
    pub vertices: usize,
    /// Text buffers rebuilt this frame; changing only position/color does not reshape.
    pub reshaped: usize,
    pub prepare_ms: f64,
}

struct CachedText { text: String, font: FontFace, align: TextAlign, size: u32, width: u32, buffer: Buffer }
impl CachedText {
    fn matches(&self, run: &TextRun) -> bool {
        self.text==run.text && self.font==run.font && self.align==run.align && self.size==run.font_size.to_bits() && self.width==run.width.to_bits()
    }
}

struct FontCache {
    fonts: FontSystem,
    body_family: String,
    title_family: String,
    /// Registered faces loaded so far: family, weight, style.
    custom: Vec<(String, Weight, Style)>,
    buffers: HashMap<u64,CachedText>,
    system_fallback: bool,
}

impl FontCache {
    fn new() -> Result<Self,String> {
        let mut db=glyphon::fontdb::Database::new();
        db.load_font_data(BODY_FONT.to_vec());
        let body_family=db.faces().next().and_then(|f|f.families.first()).map(|f|f.0.clone()).ok_or("bundled HUD body font failed to load")?;
        let body_ids:HashSet<_>=db.faces().map(|f|f.id).collect();
        db.load_font_data(TITLE_FONT.to_vec());
        let title_family=db.faces().find(|f|!body_ids.contains(&f.id)).and_then(|f|f.families.first()).map(|f|f.0.clone()).ok_or("bundled HUD title font failed to load")?;
        let mut cache=Self{fonts:FontSystem::new_with_locale_and_db("en-US".into(),db),body_family,title_family,custom:Vec::new(),buffers:HashMap::new(),system_fallback:false};
        cache.sync_registered();
        Ok(cache)
    }

    /// Load fonts registered since the last call.
    fn sync_registered(&mut self) {
        let all=registered();
        for data in &all[self.custom.len().min(all.len())..] {
            let db=self.fonts.db_mut();
            let before:HashSet<_>=db.faces().map(|f|f.id).collect();
            db.load_font_data(data.to_vec());
            let face=db.faces().find(|f|!before.contains(&f.id)).map(|f|(f.families.first().map(|n|n.0.clone()).unwrap_or_default(),f.weight,f.style));
            self.custom.push(face.unwrap_or_else(||(self.body_family.clone(),Weight::SEMIBOLD,Style::Normal)));
        }
    }

    fn buffer(&mut self, text:&str, face:FontFace, size:f32, width:f32, align:TextAlign, wrap:Wrap)->Buffer {
        let mut buffer=Buffer::new(&mut self.fonts,Metrics::new(size,size*1.2));
        buffer.set_size(Some(width.max(1.0)),None);
        buffer.set_wrap(wrap);
        let (family,weight,style)=match face {
            FontFace::Body=>(&self.body_family,Weight::SEMIBOLD,Style::Normal),
            FontFace::Title=>(&self.title_family,Weight::EXTRA_BOLD,Style::Italic),
            FontFace::Custom(k)=>self.custom.get(k as usize).map_or((&self.body_family,Weight::SEMIBOLD,Style::Normal),|c|(&c.0,c.1,c.2)),
        };
        let attrs=Attrs::new().family(Family::Name(family)).weight(weight).style(style);
        let alignment=match align {TextAlign::Left=>Align::Left,TextAlign::Center=>Align::Center,TextAlign::Right=>Align::Right};
        buffer.set_text(text,&attrs,Shaping::Advanced,Some(alignment));
        buffer.shape_until_scroll(&mut self.fonts,false);
        buffer
    }

    fn update(&mut self,runs:&[TextRun])->usize {
        if runs.iter().any(|r|matches!(r.font,FontFace::Custom(k) if k as usize>=self.custom.len())) {self.sync_registered();}
        let mut changed=0;
        for run in runs {
            if self.buffers.get(&run.id).is_some_and(|cached|cached.matches(run)){continue;}
            let buffer=self.buffer(&run.text,run.font,run.font_size,run.width,run.align,Wrap::None);
            self.buffers.insert(run.id,CachedText{text:run.text.clone(),font:run.font,align:run.align,size:run.font_size.to_bits(),width:run.width.to_bits(),buffer});
            changed+=1;
        }
        let ids:HashSet<_>=runs.iter().map(|r|r.id).collect();
        self.buffers.retain(|id,_|ids.contains(id));
        changed
    }

    fn missing_glyphs(&self)->bool {self.buffers.values().any(|b|b.buffer.layout_runs().any(|line|line.glyphs.iter().any(|g|g.glyph_id==0)))}

    fn load_fallback(&mut self) {
        // Packaged Latin is deterministic. Only consult installed fonts when an
        // actual requested Unicode glyph cannot be shaped by the two fonts.
        let mut db=self.fonts.db().clone();db.load_system_fonts();
        self.fonts=FontSystem::new_with_locale_and_db("en-US".into(),db);
        self.buffers.clear();self.system_fallback=true;
        eprintln!("HUD: enabling system font fallback for requested non-bundled glyphs");
    }
}

struct GpuImage { bind: wgpu::BindGroup, size: [u32; 2] }

pub struct HudRenderer {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    image_layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    /// Bound for plain shapes and for pictures that are not (or no longer) uploaded.
    blank: wgpu::BindGroup,
    images: HashMap<HudImage, GpuImage>,
    /// Which provided picture each id holds (by `Arc` address).
    provided: HashMap<HudImage, usize>,
    spans: Vec<Vec<(Range<u32>, Option<HudImage>)>>,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    vertex_capacity: u64,
    layers: Vec<HudLayer>,
    srgb: bool,
    text: FontCache,
    swash: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderers: Vec<TextRenderer>,
    warmed: HashSet<(FontFace,u32)>,
    size: [u32;2],
    /// To start a fresh glyph atlas when the old one is full.
    cache: Cache,
    format: wgpu::TextureFormat,
    full_said: bool,
    /// Stroked glyphs, by glyph and stroke width: their custom-glyph id
    /// and placement. The id indexes `stroke_images`.
    strokes: HashMap<(glyphon::CacheKey,u32),Option<(CustomGlyphId,swash::zeno::Placement)>>,
    stroke_images: Vec<Vec<u8>>,
    stroke_context: swash::scale::ScaleContext,
}

/// Custom glyph ids are 16 bit; past this the stroke cache (and the atlas
/// that holds them) starts over.
const MAX_STROKES: usize = 60_000;

/// Larger text (titles) is not warmed with the whole alphabet: at 4K that
/// alone can fill the glyph atlas.
const WARM_MAX_PX: f32 = 96.0;

/// A glyph's outline stroked `width` pixels each side of its edge (the
/// fill covers the inner half), as an alpha mask and its placement.
fn stroke_image(fonts:&mut FontSystem,context:&mut swash::scale::ScaleContext,key:glyphon::CacheKey,width:f32)->Option<(swash::zeno::Placement,Vec<u8>)> {
    use swash::scale::{Render,Source};
    use swash::zeno::{Cap,Format,Join,Stroke,Vector};
    let font=fonts.get_font(key.font_id,key.font_weight)?;
    let mut scaler=context.builder(font.as_swash()).size(f32::from_bits(key.font_size_bits)).hint(false).build();
    let mut stroke=Stroke::new(width*2.0);
    stroke.join(Join::Round).cap(Cap::Round);
    let image=Render::new(&[Source::Outline]).format(Format::Alpha).style(stroke)
        .offset(Vector::new(key.x_bin.as_float(),key.y_bin.as_float())).render(&mut scaler,key.glyph_id)?;
    (image.placement.width>0 && image.placement.height>0).then_some((image.placement,image.data))
}

fn new_atlas(gpu:&Gpu,cache:&Cache,format:wgpu::TextureFormat)->TextAtlas {
    TextAtlas::with_color_mode(&gpu.device,&gpu.queue,cache,format,if format.is_srgb(){ColorMode::Accurate}else{ColorMode::Web})
}

impl HudRenderer {
    pub fn new(gpu:&Gpu,format:wgpu::TextureFormat)->Result<Self,String> {
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("HUD geometry"),entries:&[
            wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::VERTEX_FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None},
        ]});
        let uniform=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("HUD display"),contents:bytemuck::cast_slice(&[1f32,1.0,0.0,0.0]),usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("HUD geometry"),layout:&layout,entries:&[wgpu::BindGroupEntry{binding:0,resource:uniform.as_entire_binding()}]});
        let image_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("HUD picture"),entries:&[
            wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None},
        ]});
        let sampler=|filter:wgpu::FilterMode,mip:wgpu::MipmapFilterMode|gpu.device.create_sampler(&wgpu::SamplerDescriptor{label:Some("HUD picture"),
            address_mode_u:wgpu::AddressMode::ClampToEdge,address_mode_v:wgpu::AddressMode::ClampToEdge,mag_filter:filter,min_filter:wgpu::FilterMode::Linear,mipmap_filter:mip,..Default::default()});
        let linear=sampler(wgpu::FilterMode::Linear,wgpu::MipmapFilterMode::Linear);
        let nearest=sampler(wgpu::FilterMode::Nearest,wgpu::MipmapFilterMode::Nearest);
        let blank=image_bind(gpu,&image_layout,&linear,&crate::picture::Picture::solid(1,1,[0,0,0,0]),"HUD blank picture");
        let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("HUD geometry"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/hud.wgsl").into())});
        let pipeline_layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("HUD geometry"),bind_group_layouts:&[Some(&layout),Some(&image_layout)],immediate_size:0});
        let pipeline=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{
            label:Some("HUD geometry"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState{module:&shader,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[Some(wgpu::VertexBufferLayout{array_stride:std::mem::size_of::<HudVertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x4,2=>Float32x4,3=>Float32x4]})]},
            primitive:wgpu::PrimitiveState::default(),depth_stencil:None,multisample:Default::default(),
            fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some("fs"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format,blend:Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),
            multiview_mask:None,cache: gpu.pipeline_cache(),
        });
        let vertex_capacity=4096;
        let vertices=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("HUD vertices"),size:vertex_capacity,usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let cache=Cache::new(&gpu.device);
        let viewport=Viewport::new(&gpu.device,&cache);
        let mut atlas=new_atlas(gpu,&cache,format);
        let renderer=TextRenderer::new(&mut atlas,&gpu.device,wgpu::MultisampleState::default(),None);
        Ok(Self{pipeline,bind,image_layout,linear,nearest,blank,images:HashMap::new(),provided:HashMap::new(),spans:Vec::new(),uniform,vertices,vertex_capacity,layers:Vec::new(),srgb:format.is_srgb(),text:FontCache::new()?,swash:SwashCache::new(),viewport,atlas,renderers:vec![renderer],warmed:HashSet::new(),size:[0;2],cache,format,full_said:false,strokes:HashMap::new(),stroke_images:Vec::new(),stroke_context:swash::scale::ScaleContext::new()})
    }

    /// Upload (or replace) the picture drawn by `HudDrawList::image` with `id`.
    /// Linear pictures get a mip chain, so covers stay smooth when drawn small.
    pub fn set_image(&mut self,gpu:&Gpu,id:HudImage,picture:&crate::picture::Picture,filter:ImageFilter) {
        let sampler=match filter {ImageFilter::Linear=>&self.linear,ImageFilter::Nearest=>&self.nearest};
        let bind=image_bind(gpu,&self.image_layout,sampler,picture,"HUD picture");
        self.images.insert(id,GpuImage{bind,size:[picture.width,picture.height]});
    }
    pub fn remove_image(&mut self,id:HudImage) {self.images.remove(&id);self.provided.remove(&id);}
    /// Uploaded size, if any.
    pub fn image_size(&self,id:HudImage)->Option<[u32;2]> {self.images.get(&id).map(|i|i.size)}

    pub fn prepare(&mut self,gpu:&Gpu,list:&HudDrawList)->Result<HudStats,String> {
        let begin=Instant::now();
        for (id,picture,filter) in &list.pictures {
            let at=std::sync::Arc::as_ptr(picture) as usize;
            if self.provided.get(id)!=Some(&at) {self.set_image(gpu,*id,picture,*filter);self.provided.insert(*id,at);}
        }
        // A failed prepare must not advertise stale overlay draw buffers.
        self.layers.clear();
        self.spans.clear();
        if list.size.contains(&0){return Err("HUD output dimensions must be positive".into());}
        if list.text.len()>MAX_RUNS || list.vertices.len()>MAX_VERTICES{return Err("HUD draw list exceeds bounded capacity".into());}
        let layers = list.layer_ranges()?;
        let mut ids=HashSet::new();
        for run in &list.text {
            if !ids.insert(run.id){return Err(format!("duplicate HUD text id {}",run.id));}
            if !run.font_size.is_finite() || !(1.0..=512.0).contains(&run.font_size) || !run.width.is_finite() || run.width<0.0 || run.text.len()>16_384 || !run.position.iter().chain(run.bounds.iter()).all(|v|v.is_finite()) || run.bounds[2]<0.0 || run.bounds[3]<0.0 {
                return Err(format!("invalid HUD text layout {}",run.id));
            }
        }
        if list.vertices.iter().any(|v|!v.position.iter().chain(v.color.iter()).chain(v.clip.iter()).chain(v.shape.iter()).all(|f|f.is_finite())){return Err("non-finite HUD geometry".into());}
        if self.size!=list.size {self.size=list.size;self.warmed.clear();}
        self.viewport.update(&gpu.queue,Resolution{width:list.size[0],height:list.size[1]});
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::cast_slice(&[list.size[0] as f32,list.size[1] as f32,if self.srgb{1.0}else{0.0},0.0]));
        let bytes=bytemuck::cast_slice(&list.vertices);
        if bytes.len() as u64>self.vertex_capacity {
            self.vertex_capacity=(bytes.len() as u64).next_power_of_two();
            self.vertices=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("HUD vertices"),size:self.vertex_capacity,usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        }
        if !bytes.is_empty(){gpu.queue.write_buffer(&self.vertices,0,bytes);}
        let mut reshaped=self.text.update(&list.text);
        if !self.text.system_fallback && self.text.missing_glyphs() {
            self.text.load_fallback();self.swash=SwashCache::new();self.warmed.clear();
            reshaped+=self.text.update(&list.text);
        }
        // A full glyph atlas (huge titles at 4K) starts a fresh one once; if
        // the text still does not fit, this frame draws without it rather
        // than failing the game.
        match self.prepare_text(gpu,list,&layers) {
            Err(glyphon::PrepareError::AtlasFull) => {
                self.atlas=new_atlas(gpu,&self.cache,self.format);
                self.renderers.clear();
                self.warmed.clear();
                self.strokes.clear();self.stroke_images.clear();
                if let Err(e)=self.prepare_text(gpu,list,&layers) {
                    if !std::mem::replace(&mut self.full_said,true) { eprintln!("HUD text left out: {e}"); }
                    self.atlas=new_atlas(gpu,&self.cache,self.format);
                    self.renderers.clear();
                    self.warmed.clear();
                    self.strokes.clear();self.stroke_images.clear();
                    let none=HudDrawList::new(list.size);
                    self.prepare_text(gpu,&none,&none.layer_ranges()?).map_err(|e|format!("HUD text preparation: {e}"))?;
                    self.renderers.resize_with(layers.len().max(1),||TextRenderer::new(&mut self.atlas,&gpu.device,wgpu::MultisampleState::default(),None));
                }
            }
            r => r.map_err(|e|format!("HUD text preparation: {e}"))?,
        }
        self.spans = layers.iter().map(|layer| list.image_spans(layer.vertices.clone())).collect();
        self.layers = layers;
        let glyphs=self.text.buffers.values().map(|b|b.buffer.layout_runs().map(|line|line.glyphs.len()).sum::<usize>()).sum();
        // Atlas remains GPU-resident. Bound the auxiliary CPU raster cache if
        // repeated window sizes or many changing Unicode names expand it.
        if self.swash.image_cache.len()>16_384 {self.swash.image_cache.clear();self.swash.outline_command_cache.clear();}
        Ok(HudStats{text_runs:list.text.len(),glyphs,vertices:list.vertices.len(),reshaped,prepare_ms:begin.elapsed().as_secs_f64()*1000.0})
    }

    /// Glyphs for every text layer into the shared atlas.
    fn prepare_text(&mut self,gpu:&Gpu,list:&HudDrawList,layers:&[HudLayer])->Result<(),glyphon::PrepareError> {
        // Release last frame's pin set, preserving the atlas's resident glyphs.
        self.atlas.trim();
        // Prepare invisible warm text first: uploads the complete instrument
        // alphabet before changing digits can introduce a rasterization hitch.
        let mut warm=Vec::new();
        for run in &list.text {
            if run.font_size<=WARM_MAX_PX && self.warmed.insert((run.font,run.font_size.to_bits())) {
                warm.push(self.text.buffer(WARM_TEXT,run.font,run.font_size,list.size[0] as f32,TextAlign::Left,Wrap::Glyph));
            }
        }
        if !warm.is_empty() {
            let areas=warm.iter().map(|buffer|TextArea{buffer,left:0.0,top:0.0,scale:1.0,bounds:TextBounds::default(),default_color:Color::rgba(255,255,255,0),custom_glyphs:&[]});
            if self.renderers.is_empty() { self.renderers.push(TextRenderer::new(&mut self.atlas,&gpu.device,wgpu::MultisampleState::default(),None)); }
            self.renderers[0].prepare(&gpu.device,&gpu.queue,&mut self.text.fonts,&mut self.atlas,&self.viewport,areas,&mut self.swash)?;
        }
        while self.renderers.len() < layers.len() {
            self.renderers.push(TextRenderer::new(&mut self.atlas,&gpu.device,wgpu::MultisampleState::default(),None));
        }
        // One retained glyph buffer per layer, one shared atlas/font cache.
        // Do not trim between layers: earlier glyphs stay pinned while later
        // overlays are prepared. Atlas growth preserves glyph coordinates.
        if self.stroke_images.len()>MAX_STROKES { return Err(glyphon::PrepareError::AtlasFull); }
        let strokes:Vec<Vec<CustomGlyph>>=list.text.iter().map(|run| list.outlines.get(&run.id).map_or_else(Vec::new,|o|self.stroke_glyphs(run.id,o))).collect();
        let images=&self.stroke_images;
        for (renderer, layer) in self.renderers.iter_mut().zip(layers) {
            let areas=list.text[layer.text.clone()].iter().zip(&strokes[layer.text.clone()]).map(|(run,stroke)| {
                let [x,y,w,h]=run.bounds;
                TextArea{buffer:&self.text.buffers[&run.id].buffer,left:run.position[0],top:run.position[1],scale:1.0,
                    bounds:TextBounds{left:x.ceil() as i32,top:y.ceil() as i32,right:(x+w).floor() as i32,bottom:(y+h).floor() as i32},
                    default_color:Color::rgba(run.color[0],run.color[1],run.color[2],run.color[3]),custom_glyphs:stroke}
            });
            // Custom glyphs (the strokes) go down before the area's letters.
            renderer.prepare_with_custom(&gpu.device,&gpu.queue,&mut self.text.fonts,&mut self.atlas,&self.viewport,areas,&mut self.swash,
                |req| images.get(req.id as usize).map(|data| RasterizedCustomGlyph{data:data.clone(),content_type:ContentType::Mask}))?;
        }
        Ok(())
    }

    /// The stroke under every letter of run `id`, as custom glyphs placed
    /// like glyphon places the letters themselves.
    fn stroke_glyphs(&mut self,id:u64,outline:&Outline)->Vec<CustomGlyph> {
        let Some(text)=self.text.buffers.get(&id) else { return Vec::new() };
        let c=outline.color;
        let color=Some(Color::rgba(c[0],c[1],c[2],c[3]));
        let mut out=Vec::new();
        for run in text.buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical=glyph.physical((0.0,0.0),1.0);
                let key=(physical.cache_key,outline.width.to_bits());
                let entry=match self.strokes.get(&key) {
                    Some(e)=>*e,
                    None=>{
                        let e=stroke_image(&mut self.text.fonts,&mut self.stroke_context,physical.cache_key,outline.width).map(|(placement,data)| {
                            self.stroke_images.push(data);
                            ((self.stroke_images.len()-1) as CustomGlyphId,placement)
                        });
                        self.strokes.insert(key,e);
                        e
                    }
                };
                let Some((gid,p))=entry else { continue };
                out.push(CustomGlyph{id:gid,left:(physical.x+p.left) as f32,top:(run.line_y.round() as i32+physical.y-p.top) as f32,
                    width:p.width as f32,height:p.height as f32,color,snap_to_physical_pixel:true,metadata:0});
            }
        }
        out
    }

    pub fn render(&self,pass:&mut wgpu::RenderPass<'_>)->Result<(),String> {
        for ((layer, renderer), spans) in self.layers.iter().zip(&self.renderers).zip(&self.spans) {
            if !layer.vertices.is_empty() {
                // Glyph rendering changes pipeline/bindings/vertex buffer.
                // Rebind all geometry state at each subsequent overlay.
                pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&self.bind,&[]);
                pass.set_vertex_buffer(0,self.vertices.slice(..));
                for (range,image) in spans {
                    let bind=image.and_then(|id|self.images.get(&id)).map_or(&self.blank,|i|&i.bind);
                    pass.set_bind_group(1,bind,&[]);pass.draw(range.clone(),0..1);
                }
            }
            renderer.render(&self.atlas,&self.viewport,pass).map_err(|e|format!("HUD rendering: {e}"))?;
        }
        Ok(())
    }
}

/// Texture with a CPU box-filtered mip chain (premultiplied, so transparent
/// edges do not darken; the shader divides alpha back out).
fn image_bind(gpu:&Gpu,layout:&wgpu::BindGroupLayout,sampler:&wgpu::Sampler,picture:&crate::picture::Picture,label:&str)->wgpu::BindGroup {
    let mut level:Vec<u8>=picture.rgba.chunks_exact(4).flat_map(|p|{let a=p[3] as u32;[(p[0] as u32*a/255) as u8,(p[1] as u32*a/255) as u8,(p[2] as u32*a/255) as u8,p[3]]}).collect();
    let (mut w,mut h)=(picture.width,picture.height);
    let mips=w.max(h).ilog2()+1;
    let texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some(label),size:wgpu::Extent3d{width:w,height:h,depth_or_array_layers:1},
        mip_level_count:mips,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,
        usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
    for mip in 0..mips {
        gpu.queue.write_texture(wgpu::TexelCopyTextureInfo{texture:&texture,mip_level:mip,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},&level,
            wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(4*w),rows_per_image:Some(h)},wgpu::Extent3d{width:w,height:h,depth_or_array_layers:1});
        if mip+1==mips {break;}
        let (nw,nh)=((w/2).max(1),(h/2).max(1));
        let mut next=vec![0u8;(nw*nh*4) as usize];
        for y in 0..nh { for x in 0..nw { for c in 0..4 {
            let mut sum=0u32;
            for (dx,dy) in [(0,0),(1,0),(0,1),(1,1)] {let sx=(x*2+dx).min(w-1);let sy=(y*2+dy).min(h-1);sum+=level[((sy*w+sx)*4+c) as usize] as u32;}
            next[((y*nw+x)*4+c) as usize]=((sum+2)/4) as u8;
        }}}
        level=next;w=nw;h=nh;
    }
    let view=texture.create_view(&Default::default());
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some(label),layout,entries:&[
        wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::TextureView(&view)},
        wgpu::BindGroupEntry{binding:1,resource:wgpu::BindingResource::Sampler(sampler)}]})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pictures_split_draws_by_image_and_plain_shapes_keep_the_blank_binding() {
        let mut list=HudDrawList::new([100,100]);let clip=[0.0,0.0,100.0,100.0];
        list.rect([0.0,0.0,10.0,10.0],[1.0;4],clip);
        list.image([0.0,0.0,10.0,10.0],HudImage(7),[0.0,0.0,1.0,1.0],[1.0;4],clip);
        list.image([10.0,0.0,10.0,10.0],HudImage(7),[1.0,0.0,-1.0,1.0],[1.0;4],clip);
        list.circle([5.0,5.0],3.0,[1.0;4],clip);
        assert_eq!(list.image_spans(0..24),vec![(0..6,None),(6..18,Some(HudImage(7))),(18..24,None)]);
        assert_eq!(list.image_spans(12..24),vec![(12..18,Some(HudImage(7))),(18..24,None)]);
        assert_eq!(list.vertices[12].shape,[1.0,0.0,0.0,3.0],"mirrored uv");
        let mut plain=HudDrawList::new([10,10]);plain.rect([0.0,0.0,1.0,1.0],[1.0;4],clip);
        assert_eq!(plain.image_spans(0..6),vec![(0..6,None)]);
    }
    #[test]
    fn gpu_pictures_draw_mirrored_tinted_and_missing_ones_stay_invisible() {
        let gpu=Gpu::new().unwrap();let format=wgpu::TextureFormat::Rgba8Unorm;
        let presenter=crate::present::Presenter::new(&gpu,format);
        let mut hud=HudRenderer::new(&gpu,format).unwrap();
        // Left half red, right half half-transparent blue.
        let rgba:Vec<u8>=(0..4*4).flat_map(|i|if i%4<2{[255,0,0,255]}else{[0,0,255,128]}).collect();
        hud.set_image(&gpu,HudImage(1),&crate::picture::Picture::new(4,4,rgba).unwrap(),ImageFilter::Nearest);
        let background=gpu.device.create_texture_with_data(&gpu.queue,&wgpu::TextureDescriptor{label:None,size:wgpu::Extent3d{width:64,height:32,depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]},
            Default::default(),&[255u8,255,255,255].repeat(64*32));
        let mut list=HudDrawList::new([64,32]);let clip=[0.0,0.0,64.0,32.0];
        list.image([0.0,0.0,16.0,16.0],HudImage(1),[0.0,0.0,1.0,1.0],[1.0;4],clip);
        list.image([16.0,0.0,16.0,16.0],HudImage(1),[1.0,0.0,-1.0,1.0],[1.0;4],clip);
        list.image([32.0,0.0,16.0,16.0],HudImage(1),[0.0,0.0,1.0,1.0],[0.0,1.0,0.0,1.0],clip);
        list.image([48.0,0.0,16.0,16.0],HudImage(99),[0.0,0.0,1.0,1.0],[1.0;4],clip);
        list.rect([0.0,16.0,64.0,16.0],[0.0,0.0,0.0,1.0],clip);
        hud.prepare(&gpu,&list).unwrap();
        let out=presenter.capture_rgba(&gpu,&background,format,Some(&hud)).unwrap();
        let px=|x:usize,y:usize|{let i=(y*64+x)*4;[out[i],out[i+1],out[i+2]]};
        assert_eq!(px(2,8),[255,0,0],"left half red");
        let blend=px(13,8);assert!(blend[2]==255 && (120..=136).contains(&blend[0]),"half blue over white: {blend:?}");
        assert_eq!(px(17,8),blend,"mirrored: blue on the left");assert_eq!(px(30,8),[255,0,0]);
        assert_eq!(px(34,8),[0,0,0],"green tint times red");
        assert_eq!(px(56,8),[255,255,255],"missing picture draws nothing");
        assert_eq!(px(8,24),[0,0,0],"plain shapes still draw");
    }
    #[test]
    fn outlined_text_gets_a_real_stroke() {
        let gpu=Gpu::new().unwrap();let format=wgpu::TextureFormat::Rgba8Unorm;
        let presenter=crate::present::Presenter::new(&gpu,format);
        let mut hud=HudRenderer::new(&gpu,format).unwrap();
        let (w,h)=(240usize,80usize);
        let background=gpu.device.create_texture_with_data(&gpu.queue,&wgpu::TextureDescriptor{label:None,size:wgpu::Extent3d{width:w as u32,height:h as u32,depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]},
            Default::default(),&[128u8,128,128,255].repeat(w*h));
        let shot=|hud:&mut HudRenderer,outline:bool| {
            let mut list=HudDrawList::new([w as u32,h as u32]);
            list.text.push(TextRun{id:7,text:"Wave 3".into(),position:[12.0,10.0],bounds:[0.0,0.0,w as f32,h as f32],font_size:48.0,color:[255;4],font:FontFace::Body,align:TextAlign::Left,width:w as f32});
            if outline { list.outline(7,4.0,[0,0,0,255]); }
            hud.prepare(&gpu,&list).unwrap();
            presenter.capture_rgba(&gpu,&background,format,Some(hud)).unwrap()
        };
        let count=|px:&[u8],f:&dyn Fn(&[u8])->bool| px.chunks(4).filter(|p|f(p)).count();
        let plain=shot(&mut hud,false);
        let stroked=shot(&mut hud,true);
        if let Ok(dir)=std::env::var("HUD_OUTLINE_PNG") { crate::write_png(std::path::Path::new(&dir),w as u32,h as u32,&stroked).unwrap(); }
        let dark=|p:&[u8]|p[0]<40&&p[1]<40&&p[2]<40;
        let white=|p:&[u8]|p[0]>230;
        assert_eq!(count(&plain,&dark),0,"no stroke without an outline");
        assert!(count(&stroked,&dark)>300,"a dark stroke around the letters: {}",count(&stroked,&dark));
        assert!(count(&stroked,&white)*10>count(&plain,&white)*7,"the white fill stays on top");
    }

    #[test]
    fn a_full_glyph_atlas_never_fails_the_frame() {
        // A small device: one huge title fills its largest glyph atlas.
        crate::gpu::TEST_MAX_TEXTURE.with(|c| c.set(Some(1024)));
        let gpu=Gpu::new().unwrap();
        crate::gpu::TEST_MAX_TEXTURE.with(|c| c.set(None));
        let mut hud=HudRenderer::new(&gpu,wgpu::TextureFormat::Rgba8Unorm).unwrap();
        let mut list=HudDrawList::new([3840,2160]);
        list.text.push(TextRun{id:1,text:"ABCDEFGHIJKLMNOPQRSTUVWXYZ".into(),position:[0.0;2],bounds:[0.0,0.0,3840.0,2160.0],font_size:400.0,color:[255;4],font:FontFace::Body,align:TextAlign::Left,width:3840.0});
        hud.prepare(&gpu,&list).expect("a full atlas leaves text out, never errors");
        list.text[0].text="Level 2".into();
        list.text[0].font_size=120.0;
        hud.prepare(&gpu,&list).expect("and the next frame draws again");
    }

    fn run()->TextRun {TextRun{id:1,text:"Zoë Ångström 120 km/h".into(),position:[0.0;2],bounds:[0.0,0.0,600.0,100.0],font_size:32.0,color:[255;4],font:FontFace::Body,align:TextAlign::Left,width:600.0}}
    #[test]
    fn registered_fonts_shape_and_measure() {
        static FACE: &[u8] = include_bytes!("../../../assets/fonts/Barlow-SemiBold.ttf");
        let face = register_font(FACE);
        assert_eq!(register_font(FACE), face, "registering twice gives the same face");
        let mut cache = FontCache::new().unwrap();
        let mut text = run(); text.font = face;
        assert_eq!(cache.update(&[text]), 1);
        assert!(!cache.missing_glyphs());
        let [w, h] = measure("ABC", face, 20.0);
        let [w2, _] = measure("ABCABC", face, 20.0);
        assert!(w > 10.0 && (w2 - 2.0 * w).abs() < 2.0 && (h - 24.0).abs() < 0.01, "{w} {w2} {h}");
    }
    #[test]
    fn polygons_and_arcs_make_whole_triangles() {
        let mut list = HudDrawList::new([100, 100]);
        list.polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], [1.0; 4], [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(list.vertices.len(), 12);
        list.arc([50.0, 50.0], 20.0, 0.0, 1.0, 4.0, [1.0; 4], [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(list.vertices.len() % 3, 0);
        assert!(list.layer_ranges().is_ok());
    }
    #[test]
    fn bundled_fonts_shape_latin_without_system_dependencies() {
        let mut cache=FontCache::new().unwrap();let mut text=run();
        for font in [FontFace::Body,FontFace::Title] {text.font=font;assert_eq!(cache.update(&[text.clone()]),1);assert!(!cache.missing_glyphs());}
        assert!(!cache.system_fallback);assert!(FONT_LICENSE.contains("SIL OPEN FONT LICENSE"));
    }
    #[test]
    fn text_cache_ignores_position_color_and_clip_but_tracks_layout_changes() {
        let mut cache=FontCache::new().unwrap();let mut text=run();
        assert_eq!(cache.update(&[text.clone()]),1);text.position=[400.0,200.0];text.color=[255,0,0,128];text.bounds[0]=300.0;
        assert_eq!(cache.update(&[text.clone()]),0);text.text.push('!');assert_eq!(cache.update(&[text.clone()]),1);
        text.width=450.0;assert_eq!(cache.update(&[text.clone()]),1);text.font_size=48.0;assert_eq!(cache.update(&[text.clone()]),1);
        text.align=TextAlign::Right;assert_eq!(cache.update(&[text]),1);cache.update(&[]);assert!(cache.buffers.is_empty());
    }
    #[test]
    fn geometry_preserves_each_players_clip_and_has_finite_degenerate_handling() {
        let mut list=HudDrawList::new([3840,2160]);let clip=[1924.0,1084.0,1916.0,1076.0];
        list.rect([1940.0,1100.0,100.0,40.0],[1.0;4],clip);list.circle([2000.0,1200.0],20.0,[1.0;4],clip);
        list.line([2000.0,1300.0],[2040.0,1300.0],3.0,[1.0;4],clip);
        list.line([0.0;2],[0.0;2],3.0,[1.0;4],clip);assert_eq!(list.vertices.len(),18);
        assert!(list.vertices.iter().all(|v|v.clip==clip && v.position.iter().all(|x|x.is_finite())));
    }

    #[test]
    fn overlay_geometry_follows_base_text_and_nested_overlay_is_last() {
        let mut list=HudDrawList::new([800,600]);
        let clip=[0.0,0.0,800.0,600.0];
        // All three rectangles and text layouts overlap deliberately. Merely
        // appending vertices/text to their old global batches cannot pass.
        for layer in 0..3 {
            if layer>0 {list.begin_overlay();}
            list.rect([0.0,0.0,600.0,100.0],[layer as f32/2.0,0.0,0.0,1.0],clip);
            let mut text=run();text.id=layer+1;list.text.push(text);
        }
        let layers=list.layer_ranges().unwrap();
        assert_eq!(layers,vec![
            HudLayer{vertices:0..6,text:0..1},
            HudLayer{vertices:6..12,text:1..2},
            HudLayer{vertices:12..18,text:2..3},
        ]);
        let mut order=Vec::new();
        for layer in &layers {
            if !layer.vertices.is_empty() {order.push(("geometry",layer.vertices.start as u64));}
            for text in &list.text[layer.text.clone()] {order.push(("text",text.id));}
        }
        assert_eq!(order,vec![("geometry",0),("text",1),("geometry",6),("text",2),("geometry",12),("text",3)]);
    }

    #[test]
    fn overlay_boundaries_preserve_normal_hud_and_cached_text() {
        let mut list=HudDrawList::new([800,600]);
        list.text.push(run());
        assert_eq!(list.layer_ranges().unwrap(),vec![HudLayer{vertices:0..0,text:0..1}]);
        let mut cache=FontCache::new().unwrap();
        assert_eq!(cache.update(&list.text),1);
        list.begin_overlay();list.begin_overlay();
        list.rect([0.0,0.0,600.0,100.0],[0.0,0.0,0.0,1.0],[0.0,0.0,800.0,600.0]);
        assert_eq!(list.layer_ranges().unwrap(),vec![
            HudLayer{vertices:0..0,text:0..1},HudLayer{vertices:0..6,text:1..1}]);
        assert_eq!(cache.update(&list.text),0,"layer boundaries must not reshape existing text");
        assert_eq!(cache.buffers.len(),1);
        let mut empty=HudDrawList::new([800,600]);empty.begin_overlay();empty.begin_overlay();
        assert!(empty.layer_ranges().unwrap().is_empty());
    }

    #[test]
    fn overlay_boundaries_reject_truncation_and_excess_layers() {
        let mut list=HudDrawList::new([800,600]);list.text.push(run());list.begin_overlay();
        list.text.clear();
        assert!(list.layer_ranges().is_err());
        let mut list=HudDrawList::new([800,600]);
        for i in 0..=MAX_LAYERS {
            list.begin_overlay();let mut text=run();text.id=i as u64;list.text.push(text);
        }
        assert!(list.layer_ranges().is_err());
    }
}
