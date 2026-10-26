//! Native-resolution HUD geometry and cached text, shared by every split view.
//! Coordinates and clips are physical pixels; all rectangles are x/y/width/height.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glyphon::{Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping, Style, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap};
use glyphon::cosmic_text::Align;
use std::{collections::{HashMap, HashSet}, ops::Range, time::Instant};
use wgpu::util::DeviceExt;

const BODY_FONT: &[u8] = include_bytes!("../../../assets/fonts/BarlowCondensed-SemiBold.ttf");
const TITLE_FONT: &[u8] = include_bytes!("../../../assets/fonts/BarlowCondensed-ExtraBoldItalic.ttf");
/// Barlow is packaged under SIL OFL 1.1; keep this notice with distributions.
pub const FONT_LICENSE: &str = include_str!("../../../assets/fonts/OFL-BarlowCondensed.txt");
const WARM_TEXT: &str = "0123456789:-+. /()<> ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz";
const MAX_RUNS: usize = 512;
const MAX_VERTICES: usize = 131_072;
const MAX_LAYERS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFace { Body, Title }
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct HudVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub clip: [f32; 4],
    // 0: plain triangle; 1: circle (center x/y, radius); 2: line (distance, half-width).
    pub shape: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct HudDrawList {
    pub size: [u32; 2],
    pub vertices: Vec<HudVertex>,
    pub text: Vec<TextRun>,
    overlay_boundaries: Vec<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HudLayer { vertices: Range<u32>, text: Range<usize> }

impl HudDrawList {
    pub fn new(size: [u32; 2]) -> Self { Self { size, vertices: Vec::new(), text: Vec::new(), overlay_boundaries: Vec::new() } }

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
        for i in [0, 1, 2, 0, 2, 3] { self.vertices.push(HudVertex { position: points[i], color, clip, shape: shapes[i] }); }
    }

    pub fn rect(&mut self, rect: [f32; 4], color: [f32; 4], clip: [f32; 4]) {
        let [x,y,w,h]=rect;
        if w<=0.0 || h<=0.0 {return;}
        self.quad([[x,y],[x+w,y],[x+w,y+h],[x,y+h]], [[0.0;4];4], color, clip);
    }

    pub fn triangle(&mut self, points: [[f32; 2]; 3], color: [f32; 4], clip: [f32; 4]) {
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
        Ok(Self{fonts:FontSystem::new_with_locale_and_db("en-US".into(),db),body_family,title_family,buffers:HashMap::new(),system_fallback:false})
    }

    fn buffer(&mut self, text:&str, face:FontFace, size:f32, width:f32, align:TextAlign, wrap:Wrap)->Buffer {
        let mut buffer=Buffer::new(&mut self.fonts,Metrics::new(size,size*1.2));
        buffer.set_size(Some(width.max(1.0)),None);
        buffer.set_wrap(wrap);
        let (family,weight,style)=match face {
            FontFace::Body=>(&self.body_family,Weight::SEMIBOLD,Style::Normal),
            FontFace::Title=>(&self.title_family,Weight::EXTRA_BOLD,Style::Italic),
        };
        let attrs=Attrs::new().family(Family::Name(family)).weight(weight).style(style);
        let alignment=match align {TextAlign::Left=>Align::Left,TextAlign::Center=>Align::Center,TextAlign::Right=>Align::Right};
        buffer.set_text(text,&attrs,Shaping::Advanced,Some(alignment));
        buffer.shape_until_scroll(&mut self.fonts,false);
        buffer
    }

    fn update(&mut self,runs:&[TextRun])->usize {
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

pub struct HudRenderer {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
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
}

impl HudRenderer {
    pub fn new(gpu:&Gpu,format:wgpu::TextureFormat)->Result<Self,String> {
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("HUD geometry"),entries:&[
            wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::VERTEX_FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None},
        ]});
        let uniform=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("HUD display"),contents:bytemuck::cast_slice(&[1f32,1.0,0.0,0.0]),usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("HUD geometry"),layout:&layout,entries:&[wgpu::BindGroupEntry{binding:0,resource:uniform.as_entire_binding()}]});
        let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("HUD geometry"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/hud.wgsl").into())});
        let pipeline_layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("HUD geometry"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
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
        let mut atlas=TextAtlas::with_color_mode(&gpu.device,&gpu.queue,&cache,format,if format.is_srgb(){ColorMode::Accurate}else{ColorMode::Web});
        let renderer=TextRenderer::new(&mut atlas,&gpu.device,wgpu::MultisampleState::default(),None);
        Ok(Self{pipeline,bind,uniform,vertices,vertex_capacity,layers:Vec::new(),srgb:format.is_srgb(),text:FontCache::new()?,swash:SwashCache::new(),viewport,atlas,renderers:vec![renderer],warmed:HashSet::new(),size:[0;2]})
    }

    pub fn prepare(&mut self,gpu:&Gpu,list:&HudDrawList)->Result<HudStats,String> {
        let begin=Instant::now();
        // A failed prepare must not advertise stale overlay draw buffers.
        self.layers.clear();
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
        // Release last frame's pin set, preserving the atlas's resident glyphs.
        self.atlas.trim();
        // Prepare invisible warm text first: uploads the complete instrument
        // alphabet before changing digits can introduce a rasterization hitch.
        let mut warm=Vec::new();
        for run in &list.text {
            if self.warmed.insert((run.font,run.font_size.to_bits())) {
                warm.push(self.text.buffer(WARM_TEXT,run.font,run.font_size,list.size[0] as f32,TextAlign::Left,Wrap::Glyph));
            }
        }
        if !warm.is_empty() {
            let areas=warm.iter().map(|buffer|TextArea{buffer,left:0.0,top:0.0,scale:1.0,bounds:TextBounds::default(),default_color:Color::rgba(255,255,255,0),custom_glyphs:&[]});
            self.renderers[0].prepare(&gpu.device,&gpu.queue,&mut self.text.fonts,&mut self.atlas,&self.viewport,areas,&mut self.swash).map_err(|e|format!("HUD font warmup: {e}"))?;
        }
        while self.renderers.len() < layers.len() {
            self.renderers.push(TextRenderer::new(&mut self.atlas,&gpu.device,wgpu::MultisampleState::default(),None));
        }
        // One retained glyph buffer per layer, one shared atlas/font cache.
        // Do not trim between layers: earlier glyphs stay pinned while later
        // overlays are prepared. Atlas growth preserves glyph coordinates.
        for (renderer, layer) in self.renderers.iter_mut().zip(&layers) {
            let areas=list.text[layer.text.clone()].iter().map(|run| {
                let [x,y,w,h]=run.bounds;
                TextArea{buffer:&self.text.buffers[&run.id].buffer,left:run.position[0],top:run.position[1],scale:1.0,
                    bounds:TextBounds{left:x.ceil() as i32,top:y.ceil() as i32,right:(x+w).floor() as i32,bottom:(y+h).floor() as i32},
                    default_color:Color::rgba(run.color[0],run.color[1],run.color[2],run.color[3]),custom_glyphs:&[]}
            });
            renderer.prepare(&gpu.device,&gpu.queue,&mut self.text.fonts,&mut self.atlas,&self.viewport,areas,&mut self.swash).map_err(|e|format!("HUD text preparation: {e}"))?;
        }
        self.layers = layers;
        let glyphs=self.text.buffers.values().map(|b|b.buffer.layout_runs().map(|line|line.glyphs.len()).sum::<usize>()).sum();
        // Atlas remains GPU-resident. Bound the auxiliary CPU raster cache if
        // repeated window sizes or many changing Unicode names expand it.
        if self.swash.image_cache.len()>16_384 {self.swash.image_cache.clear();self.swash.outline_command_cache.clear();}
        Ok(HudStats{text_runs:list.text.len(),glyphs,vertices:list.vertices.len(),reshaped,prepare_ms:begin.elapsed().as_secs_f64()*1000.0})
    }

    pub fn render(&self,pass:&mut wgpu::RenderPass<'_>)->Result<(),String> {
        for (layer, renderer) in self.layers.iter().zip(&self.renderers) {
            if !layer.vertices.is_empty() {
                // Glyph rendering changes pipeline/bindings/vertex buffer.
                // Rebind all geometry state at each subsequent overlay.
                pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&self.bind,&[]);
                pass.set_vertex_buffer(0,self.vertices.slice(..));pass.draw(layer.vertices.clone(),0..1);
            }
            renderer.render(&self.atlas,&self.viewport,pass).map_err(|e|format!("HUD rendering: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run()->TextRun {TextRun{id:1,text:"Zoë Ångström 120 km/h".into(),position:[0.0;2],bounds:[0.0,0.0,600.0,100.0],font_size:32.0,color:[255;4],font:FontFace::Body,align:TextAlign::Left,width:600.0}}
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
