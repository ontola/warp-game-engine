//! Bounded declarative captures over one real stage. Tick positions are reached
//! by the unchanged 120 Hz simulation, never by teleporting cars or rebuilding.
use glam::Vec3;
use render::Camera;
use std::{collections::{BTreeSet,HashMap},path::{Path,PathBuf}};
use toml::Value;

#[derive(Clone,Debug)]
pub struct Section {pub name:String,pub start_tick:u64,pub frames:u64,pub warmup:u64,pub cameras:Vec<Camera>,pub sun:Option<String>,pub atmosphere:Option<String>}
#[derive(Clone,Debug)]
pub struct Scenario {
    pub name:String,pub path:PathBuf,pub ticks_per_frame:u64,pub sections:Vec<Section>,pub total_frames:u64,
    pub capture_frames:BTreeSet<u64>,pub capture_dir:Option<PathBuf>,pub capture:Capture,scene_args:Vec<String>,pub views:usize,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum CaptureFormat {Jpeg,Png}
#[derive(Clone,Copy,Debug)]
pub struct Capture {pub format:CaptureFormat,pub max_width:u32,pub quality:u8}
impl Default for Capture {fn default()->Self {Self {format:CaptureFormat::Jpeg,max_width:1600,quality:85}}}
impl Capture {
    pub fn name(self)->&'static str {match self.format {CaptureFormat::Jpeg=>"jpeg",CaptureFormat::Png=>"png"}}
    fn extension(self)->&'static str {match self.format {CaptureFormat::Jpeg=>"jpg",CaptureFormat::Png=>"png"}}
    pub fn write(self,path:&Path,width:u32,height:u32,rgba:&[u8])->Result<(),String> {
        if self.format==CaptureFormat::Png{return render::write_png(path,width,height,rgba);}
        let file=std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e|format!("scenario JPEG: {e}"))?;
        let mut writer=std::io::BufWriter::new(file);self.encode_jpeg(&mut writer,width,height,rgba)?;
        std::io::Write::flush(&mut writer).map_err(|e|format!("scenario JPEG flush: {e}"))
    }
    fn encode_jpeg(self,writer:impl std::io::Write,width:u32,height:u32,rgba:&[u8])->Result<(),String> {
        let pixels=width as usize*height as usize;
        if width==0||height==0||rgba.len()!=pixels*4{return Err("scenario capture RGBA dimensions mismatch".into());}
        let mut rgb=Vec::with_capacity(pixels*3);for p in rgba.chunks_exact(4){rgb.extend_from_slice(&p[..3]);}
        image::codecs::jpeg::JpegEncoder::new_with_quality(writer,self.quality).encode(&rgb,width,height,image::ExtendedColorType::Rgb8)
            .map_err(|e|format!("scenario JPEG encoding: {e}"))
    }
    fn metadata(self)->serde_json::Value {serde_json::json!({"format":self.name(),"max_width":self.max_width,"quality":if self.format==CaptureFormat::Jpeg{Some(self.quality)}else{None}})}
}
#[derive(Clone,Copy)]
pub struct Frame<'a> {pub section:&'a Section,pub index:usize,pub local_frame:u64,pub target_tick:u64,pub warmup:bool}
impl Scenario {
    pub fn load(path:&Path)->Result<Self,String>{Self::parse(&std::fs::read_to_string(path).map_err(|e|format!("scenario {}: {e}",path.display()))?,path)}
    fn parse(text:&str,path:&Path)->Result<Self,String> {
        let value:Value=toml::from_str(text).map_err(|e|format!("scenario TOML: {e}"))?;let root=table(&value,"scenario")?;
        keys(root,&["schema","name","ticks_per_frame","scene","sections","capture_frames","capture_dir","capture"])?;
        if integer(root.get("schema"),1,"schema")?!=1{return Err("scenario schema must be 1".into());}
        let name=string(root.get("name"),"scenario","name")?;
        let ticks_per_frame=integer(root.get("ticks_per_frame"),1,"ticks_per_frame")?;
        if ticks_per_frame>8{return Err("scenario ticks_per_frame must be 0..8".into());}
        let mut capture=Capture::default();
        if let Some(value)=root.get("capture") {
            let row=table(value,"capture")?;keys(row,&["format","max_width","quality"])?;
            capture.format=match string(row.get("format"),"jpeg","capture.format")?.as_str(){"jpeg"|"jpg"=>CaptureFormat::Jpeg,"png"=>CaptureFormat::Png,_=>return Err("capture.format must be jpeg or png".into())};
            let max_width=integer(row.get("max_width"),1600,"capture.max_width")?;if max_width>7680{return Err("capture.max_width must be 0..7680; 0 keeps native resolution".into());}capture.max_width=max_width as u32;
            let quality=integer(row.get("quality"),85,"capture.quality")?;if !(1..=100).contains(&quality){return Err("capture.quality must be 1..100".into());}capture.quality=quality as u8;
        }
        let mut scene_args=vec![];let mut views=1;
        if let Some(scene)=root.get("scene") {
            let scene=table(scene,"scene")?;keys(scene,&["seed","km","weather","views","size","upscale","hud"])?;
            views=integer(scene.get("views"),1,"scene.views")? as usize;if !(1..=4).contains(&views){return Err("scenario scene.views must be 1..4".into());}
            for key in ["seed","km","weather","views","size","upscale","hud"] {if let Some(value)=scene.get(key) {
                let value=match value {Value::String(s)=>s.clone(),Value::Integer(v)=>v.to_string(),Value::Float(v) if v.is_finite()=>v.to_string(),Value::Boolean(v) if key=="hud"=>if *v{"on"}else{"off"}.into(),_=>return Err(format!("invalid scene.{key}"))};
                scene_args.extend([format!("--{key}"),value]);
            }}
        }
        let rows=root.get("sections").and_then(Value::as_array).ok_or("scenario requires [[sections]]")?;
        if rows.is_empty()||rows.len()>16{return Err("scenario needs 1..16 sections".into());}
        let mut sections=vec![];let mut names=BTreeSet::new();let mut total=0u64;let mut expected_tick=0u64;
        for (index,row) in rows.iter().enumerate() {
            let row=table(row,"section")?;keys(row,&["name","start_tick","frames","warmup","camera","cameras","sun","atmosphere"])?;
            let name=string(row.get("name"),"","section.name")?;if name.is_empty()||name.len()>80||!names.insert(name.clone()){return Err("section names must be unique, nonempty, <=80 bytes".into());}
            let frames=integer(row.get("frames"),0,"section.frames")?;let warmup=integer(row.get("warmup"),0,"section.warmup")?;
            if frames==0||frames>4096||warmup>=frames{return Err("section needs 1..4096 frames and warmup < frames".into());}
            total+=frames;if total>4096{return Err("scenario total exceeds 4096 frames".into());}
            let start_tick=integer(row.get("start_tick"),expected_tick,"section.start_tick")?;
            if start_tick>45000 || (index>0&&start_tick!=expected_tick){return Err(format!("section {name}: start_tick must continue at {expected_tick} (no rewind/teleport); initial maximum 45000"));}
            expected_tick=start_tick+frames*ticks_per_frame;
            let mut cameras=vec![];
            if row.contains_key("camera")&&row.contains_key("cameras"){return Err("choose camera or cameras, not both".into());}
            if let Some(c)=row.get("camera"){cameras=vec![camera(c)?;views];}
            if let Some(c)=row.get("cameras") {cameras=c.as_array().ok_or("cameras must be an array")?.iter().map(camera).collect::<Result<_,_>>()?;
                if cameras.len()!=views{return Err("cameras must contain exactly scene.views entries".into());}}
            let sun=row.get("sun").map(|v|string(Some(v),"","sun")).transpose()?;
            let atmosphere=row.get("atmosphere").map(|v|string(Some(v),"","atmosphere")).transpose()?;
            if let Some(v)=&sun {crate::procedural_sky::DayTime::parse(v)?;}
            if let Some(v)=&atmosphere {crate::procedural_sky::WeatherLook::parse(v)?;}
            sections.push(Section {name,start_tick,frames,warmup,cameras,sun,atmosphere});
        }
        let mut capture_frames=BTreeSet::new();
        if let Some(v)=root.get("capture_frames") {for v in v.as_array().ok_or("capture_frames must be an array")? {
            let frame=integer(Some(v),0,"capture_frames")?;if frame==0||frame>total||!capture_frames.insert(frame){return Err("capture_frames must be unique one-based rendered frame IDs within the scenario".into());}
        }}
        let capture_dir=root.get("capture_dir").map(|v|string(Some(v),"","capture_dir")).transpose()?.map(|p|path.parent().unwrap_or(Path::new(".")).join(p));
        Ok(Self {name,path:path.to_path_buf(),ticks_per_frame,sections,total_frames:total,capture_frames,capture_dir,capture,scene_args,views})
    }
    pub fn arguments(&self,raw:&[String])->Result<Vec<String>,String> {
        for flag in ["--seed","--km","--weather","--views","--size","--upscale","--hud"] {
            if raw.iter().any(|a|a==flag){return Err(format!("{flag} belongs in the scenario [scene] table"));}
        }
        if raw.iter().any(|a|matches!(a.as_str(),"--smoke-ticks"|"--smoke-next-stage")){return Err("scenario uses its own ticks; no smoke-ticks or next-stage override".into());}
        if raw.iter().any(|a|a=="--capture")&&!raw.iter().any(|a|a=="--smoke-render"){return Err("--capture is the final hidden smoke image; use scenario capture_frames/capture_dir for visible runs".into());}
        if let Some(index)=raw.iter().position(|a|a=="--smoke-render") {
            if raw.get(index+1).and_then(|v|v.parse::<u64>().ok())!=Some(self.total_frames){return Err("scenario hidden --smoke-render must equal total scenario frames".into());}
        }
        let mut args=raw.to_vec();args.extend(self.scene_args.clone());
        if !raw.iter().any(|a|a=="--sound"){args.extend(["--sound".into(),"off".into()]);}Ok(args)
    }
    pub fn frame(&self,index:u64)->Option<Frame<'_>> {
        let mut first=0;for (i,section) in self.sections.iter().enumerate(){if index<first+section.frames {
            let local=index-first;return Some(Frame {section,index:i,local_frame:local+1,target_tick:section.start_tick+local*self.ticks_per_frame,warmup:local<section.warmup});
        }first+=section.frames;}None
    }
    pub fn cameras(&self,index:u64,fallback:&[Camera])->Vec<Camera>{self.frame(index).filter(|f|!f.section.cameras.is_empty()).map(|f|f.section.cameras.clone()).unwrap_or_else(||fallback.to_vec())}
    pub fn capture_path(&self,frame_id:u64)->Option<PathBuf>{self.capture_frames.contains(&frame_id).then(||self.capture_dir.as_ref().unwrap().join(format!("frame-{frame_id:06}.{}",self.capture.extension())))}
    pub fn metadata(&self)->serde_json::Value {serde_json::json!({"name":self.name,"file":self.path,"physics_hz":120,"car_substeps":sideways::car::SUBSTEPS,
        "ticks_per_frame":self.ticks_per_frame,"views":self.views,"total_frames":self.total_frames,"capture_frames":self.capture_frames,"capture":self.capture.metadata(),
        "sections":self.sections.iter().map(|s|serde_json::json!({"name":s.name,"start_tick":s.start_tick,"frames":s.frames,"warmup":s.warmup,"sun":s.sun,"atmosphere":s.atmosphere,"fixed_cameras":s.cameras.len()})).collect::<Vec<_>>(),
        "scope":"One generated stage; unchanged AI/120Hz physics; initial ticks fast-forwarded before visuals. No car teleport, stage regeneration or renderer rebuild between sections. Wheel effects start at initial tick, not historical trail reconstruction."})}
}

pub struct Runtime {pub config:Scenario,pub skies:Vec<Option<crate::procedural_sky::PreparedSky>>,pub applied_section:Option<usize>}
impl Runtime {
    pub fn prepare(config:Scenario)->Result<Self,String> {
        if !config.capture_frames.is_empty()&&config.capture_dir.is_none(){return Err("capture_frames requires capture_dir or --scenario-output".into());}
        if let Some(dir)=&config.capture_dir {std::fs::create_dir_all(dir).map_err(|e|format!("scenario capture directory: {e}"))?;}
        for frame in &config.capture_frames {let path=config.capture_path(*frame).unwrap();if path.exists()||path.with_extension("json").exists(){return Err(format!("scenario capture already exists: {}",path.display()));}}
        let mut skies=vec![];let mut settings=crate::procedural_sky::LiveSettings::default();let mut active=false;
        let mut cache=render::atmosphere::AtmosphereCache::default();let mut baked=HashMap::new();
        for section in &config.sections {
            if let Some(sun)=&section.sun {settings=settings.changed("sky_time",&sun.clone().into())?;active=true;}
            if let Some(weather)=&section.atmosphere {settings=settings.changed("sky_weather",&weather.clone().into())?;active=true;}
            if active {let key=(settings.time.key(),settings.atmosphere.key());
                let sky=if let Some(sky)=baked.get(&key) {crate::procedural_sky::PreparedSky::clone(sky)} else {
                    let sky=crate::procedural_sky::bake_live(0,settings,&mut cache,render::atmosphere::BakeQuality::default())?;baked.insert(key,sky.clone());sky};skies.push(Some(sky));
            }else{skies.push(None);}
        }
        Ok(Self {config,skies,applied_section:None})
    }
}

pub fn advance_to(race:&mut sideways::race::Race,target:u64,mut on_tick:impl FnMut(&sideways::race::Race))->Result<u32,String> {
    if target<race.frame{return Err("scenario cannot rewind physics".into());}
    let count=target-race.frame;if count>45000{return Err("scenario tick advance exceeds 45000".into());}
    while race.frame<target {
        if race.phase=="results"&&race.results<=1.0/120.0{return Err("scenario reaches a stage transition; shorten the tick range to retain its single-stage contract".into());}
        race.step_with_inputs(&mut kern::Timings::default(),&[]);on_tick(race);
    }Ok(count as u32)
}
fn table<'a>(v:&'a Value,name:&str)->Result<&'a toml::Table,String>{v.as_table().ok_or_else(||format!("{name} must be a table"))}
fn keys(t:&toml::Table,allowed:&[&str])->Result<(),String>{if let Some(k)=t.keys().find(|k|!allowed.contains(&k.as_str())){Err(format!("unknown scenario key {k}"))}else{Ok(())}}
fn integer(v:Option<&Value>,default:u64,name:&str)->Result<u64,String>{v.map(|v|v.as_integer().filter(|v|*v>=0).map(|v|v as u64).ok_or_else(||format!("{name} must be a nonnegative integer"))).unwrap_or(Ok(default))}
fn string(v:Option<&Value>,default:&str,name:&str)->Result<String,String>{v.map(|v|v.as_str().map(str::to_owned).ok_or_else(||format!("{name} must be text"))).unwrap_or_else(||Ok(default.into()))}
fn number(v:&Value)->Result<f32,String>{let n=v.as_float().or_else(||v.as_integer().map(|v|v as f64)).ok_or("camera values must be numbers")?;if !n.is_finite()||n.abs()>1e6{return Err("camera values must be finite and within +/-1000000".into());}Ok(n as f32)}
fn vector(v:Option<&Value>,name:&str)->Result<Vec3,String>{let a=v.and_then(Value::as_array).filter(|v|v.len()==3).ok_or_else(||format!("camera {name} requires 3 numbers"))?;Ok(Vec3::new(number(&a[0])?,number(&a[1])?,number(&a[2])?))}
fn camera(value:&Value)->Result<Camera,String>{let t=table(value,"camera")?;keys(t,&["position","target","fov_y_degrees"])?;let pos=vector(t.get("position"),"position")?;let target=vector(t.get("target"),"target")?;
    let dir=target-pos;if dir.length_squared()<0.01||dir.normalize().cross(Vec3::Y).length_squared()<1e-6{return Err("camera target must differ from position and not point exactly vertical".into());}
    let fov=t.get("fov_y_degrees").map(number).transpose()?.unwrap_or(64.0);if !(15.0..=110.0).contains(&fov){return Err("camera FOV must be 15..110 degrees".into());}
    Ok(Camera {pos,target,fov_y:fov.to_radians(),near:0.1})}

#[cfg(test)] mod tests {
    use super::*;
    fn parse(s:&str)->Result<Scenario,String>{Scenario::parse(s,Path::new("scenarios/test.toml"))}
    #[test] fn exact_sections_keep_global_frame_ids_and_contiguous_ticks() {
        let s=parse("ticks_per_frame=2\n[[sections]]\nname='a'\nstart_tick=1200\nframes=4\nwarmup=2\n[[sections]]\nname='b'\nframes=3\nwarmup=1").unwrap();
        assert_eq!(s.frame(0).unwrap().target_tick,1200);assert_eq!(s.frame(3).unwrap().target_tick,1206);
        assert_eq!(s.frame(4).unwrap().target_tick,1208);assert_eq!(s.frame(4).unwrap().local_frame,1);assert!(s.frame(4).unwrap().warmup);
        assert!(!s.frame(5).unwrap().warmup);assert!(s.frame(7).is_none());
    }
    #[test] fn rejects_rewinds_invalid_cameras_capture_indices_and_unbounded_runs() {
        for s in ["[[sections]]\nname='a'\nframes=4097", "[[sections]]\nname='a'\nframes=2\nwarmup=2", "capture_frames=[0]\ncapture_dir='out'\n[[sections]]\nname='a'\nframes=2", "[[sections]]\nname='a'\nframes=2\n[[sections]]\nname='b'\nstart_tick=0\nframes=2", "[[sections]]\nname='a'\nframes=2\ncamera={position=[0,0,0],target=[0,0,0]}"] {assert!(parse(s).is_err(),"{s}");}
    }
    #[test] fn frozen_views_and_capture_paths_are_explicit_and_scene_overrides_rejected() {
        let s=parse("ticks_per_frame=0\ncapture_frames=[2]\ncapture_dir='out'\n[scene]\nviews=4\n[[sections]]\nname='a'\nstart_tick=45\nframes=2\ncamera={position=[0,2,4],target=[0,1,0]}").unwrap();
        assert_eq!(s.cameras(0,&[]).len(),4);assert_eq!(s.frame(1).unwrap().target_tick,45);
        assert_eq!(s.capture_path(2).unwrap(),Path::new("scenarios/out/frame-000002.jpg"));assert!(s.capture_path(1).is_none());
        assert!(s.arguments(&["--views".into(),"1".into()]).is_err());
    }
    #[test] fn initial_fast_forward_uses_exact_unchanged_race_ticks() {
        use sideways::{race::{Race,Settings},spec::Cars};
        let settings=Settings {stage_km:0.05,rivals:0,..Default::default()};
        let mut a=Race::new(71,settings.clone(),Cars::load(None).unwrap());let mut b=Race::new(71,settings,Cars::load(None).unwrap());
        assert_eq!(advance_to(&mut a,24,|_|{}).unwrap(),24);for _ in 0..24 {b.step_with_inputs(&mut kern::Timings::default(),&[]);}
        assert_eq!(a.frame,b.frame);assert_eq!(a.rng,b.rng);assert_eq!(a.clock,b.clock);
        for (a,b) in super::super::poses(&a).iter().zip(super::super::poses(&b)){assert_eq!(a.pos,b.pos);assert_eq!(a.rot,b.rot);}
        assert!(advance_to(&mut a,23,|_|{}).is_err());
    }
    #[test] fn checked_in_scenarios_match_the_real_cli_schema() {
        let root=sideways::game_assets::source_root().join("scenarios");
        for name in ["noon-foliage.toml","sunrise-rocks.toml","fourplayer-route.toml"] {
            let scenario=Scenario::load(&root.join(name)).unwrap();
            let arguments=scenario.arguments(&["play".into()]).unwrap();
            let actual=super::super::args(&arguments).unwrap();
            assert_eq!(actual.views,scenario.views);assert!(scenario.total_frames<=4096);
            assert_eq!(actual.smoke_frames,0,"measured scenario defaults visible");
        }
    }
    #[test] fn captures_default_to_bounded_jpeg_and_native_png_is_explicit() {
        let s=parse("capture_frames=[1]\ncapture_dir='out'\n[[sections]]\nname='a'\nframes=2").unwrap();
        assert_eq!(s.capture.format,CaptureFormat::Jpeg);assert_eq!(s.capture.max_width,1600);assert_eq!(s.capture.quality,85);
        let s=parse("capture_frames=[1]\ncapture_dir='out'\n[capture]\nformat='png'\nmax_width=0\n[[sections]]\nname='a'\nframes=2").unwrap();
        assert_eq!(s.capture.format,CaptureFormat::Png);assert_eq!(s.capture.max_width,0);assert_eq!(s.capture_path(1).unwrap().extension().unwrap(),"png");
        for setting in ["format='gif'","quality=0","quality=101","max_width=-1","max_width=7681"] {
            assert!(parse(&format!("[capture]\n{setting}\n[[sections]]\nname='a'\nframes=2")).is_err());
        }
    }
    #[test] fn jpeg_output_is_real_rgb_with_exact_supplied_preview_dimensions() {
        let rgba=[30u8,100,180,255].repeat(16*9);let mut encoded=Vec::new();
        Capture::default().encode_jpeg(&mut encoded,16,9,&rgba).unwrap();
        assert_eq!(&encoded[..2],&[0xff,0xd8]);let image=image::load_from_memory(&encoded).unwrap();assert_eq!([image.width(),image.height()],[16,9]);
        let pixel=image.to_rgb8().get_pixel(8,4).0;for (actual,expected) in pixel.into_iter().zip([30i16,100,180]) {assert!((actual as i16-expected).abs()<5);}
        assert!(Capture::default().encode_jpeg(Vec::new(),16,9,&rgba[..rgba.len()-1]).is_err());
    }
}
