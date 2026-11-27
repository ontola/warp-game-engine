//! Shared graphics modifiers: the simulation and collision do not change.
//! PC: --graphics full|low|minimum (or GG_GRAPHICS), or Warp's graphics level
//! (WARP_GRAPHICS / ?graphics=high|medium|low, see app::graphics), else the Settings choice
//! (`graphics_choice` in settings.json), else AUTO: what a start-menu probe
//! measured on this machine, screen and build (`graphics-probe.json`), else
//! by chip until the probe has run. Quest selects minimum. Without
//! an override the game may switch between rounds (`switch`), rebuilding its
//! visuals.
use std::sync::{Mutex, OnceLock};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Graphics {
    pub stadium: bool,
    pub minimal_forward: bool,
    pub characters: bool,
    pub sky_occlusion: bool,
    pub simple_blocks: bool,
    pub simple_guns: bool,
    pub simple_lighting: bool,
    /// Stone detail on blocks in the minimum renderer.
    pub stone_detail: bool,
    /// The character models as rigid pieces per bone: animated without the
    /// skinning the minimum renderer lacks.
    pub rigid_characters: bool,
    /// Level of detail for the skinned Warp Humans (`Character::simplify`):
    /// about this share of their triangles, within `character_error` metres.
    /// 1 is the full mesh.
    pub character_keep: f32,
    pub character_error: f32,
    pub debris_capacity: usize,
    pub debris_divisor: u32,
    pub debris_size: f32,
    pub particles: usize,
    pub lod_pixels: f32,
}
impl Graphics {
    pub const FULL: Self=Self {minimal_forward:false,characters:true,stadium:true,sky_occlusion:true,simple_blocks:false,simple_guns:false,simple_lighting:false,stone_detail:false,rigid_characters:false,character_keep:0.5,character_error:0.004,debris_capacity:4096,debris_divisor:1,debris_size:1.0,particles:768,lod_pixels:1.0};
    pub const LOW: Self=Self {minimal_forward:false,characters:true,stadium:false,sky_occlusion:false,simple_blocks:true,simple_guns:true,simple_lighting:true,stone_detail:false,rigid_characters:false,character_keep:0.2,character_error:0.008,debris_capacity:128,debris_divisor:16,debris_size:1.6,particles:128,lod_pixels:4.0};
    pub const MINIMUM:Self=Self {minimal_forward:true,characters:false,stone_detail:true,rigid_characters:true,debris_capacity:0,particles:32,lod_pixels:16.0,..Self::LOW};
    /// A profile name, optionally with switches for measuring one part at a
    /// time: `full:sky_occlusion=0,stadium=1`.
    pub fn parse(s:&str)->Result<Self,String> {
        let (name,switches)=s.split_once(':').unwrap_or((s,""));
        let mut g=match name {"full"=>Self::FULL,"low"=>Self::LOW,"minimum"=>Self::MINIMUM,_=>return Err(format!("unknown graphics profile {s}; use full, low or minimum"))};
        for kv in switches.split(',').filter(|kv|!kv.is_empty()) {
            let (k,v)=kv.split_once('=').ok_or(format!("graphics switch {kv}: use NAME=0|1"))?;
            let on=v=="1";
            match k {"stadium"=>g.stadium=on,"characters"=>g.characters=on,"sky_occlusion"=>g.sky_occlusion=on,"simple_blocks"=>g.simple_blocks=on,
                "simple_guns"=>g.simple_guns=on,"stone_detail"=>g.stone_detail=on,"rigid_characters"=>g.rigid_characters=on,"simple_lighting"=>g.simple_lighting=on,"minimal_forward"=>g.minimal_forward=on,
                _=>return Err(format!("unknown graphics switch {k}"))}
        }
        Ok(g)
    }
    pub fn debris_count(self,count:u32)->u32 {count.div_ceil(self.debris_divisor).min(self.debris_capacity as u32)}
    pub fn name(self)->&'static str {if self==Self::FULL {"full"} else if self==Self::LOW {"low"} else {"minimum"}}
    /// One step cheaper, if there is one.
    /// The profile for Warp's graphics level (app::graphics): high FULL,
    /// medium LOW, low MINIMUM.
    pub fn for_level(level:app::graphics::Level)->Self {
        match level {app::graphics::Level::High=>Self::FULL,app::graphics::Level::Medium=>Self::LOW,app::graphics::Level::Low=>Self::MINIMUM}
    }
    pub fn lower(self)->Option<Self> {if self==Self::FULL {Some(Self::LOW)} else if self==Self::LOW {Some(Self::MINIMUM)} else {None}}
}
static GRAPHICS:Mutex<Option<Graphics>>=Mutex::new(None);
/// --graphics or GG_GRAPHICS chose the profile: the game does not change it.
static FIXED:OnceLock<bool>=OnceLock::new();
/// A fixed profile (--graphics, Quest), before the scene is created.
pub fn configure(profile:Graphics)->Result<(),String> {
    let mut current=GRAPHICS.lock().unwrap();
    if let Some(current)=*current {return if current==profile {Ok(())}else{Err("graphics profile is fixed when the scene is created; restart to change it".into())};}
    *current=Some(profile);
    let _=FIXED.set(true);
    Ok(())
}
pub fn get()->Graphics {*GRAPHICS.lock().unwrap().get_or_insert_with(initial)}
/// Whether --graphics or GG_GRAPHICS set the profile (Settings and the
/// automatic downgrade then leave it alone).
pub fn fixed()->bool {*FIXED.get_or_init(||std::env::var("GG_GRAPHICS").is_ok_and(|s|Graphics::parse(&s).is_ok()) || app::graphics::chosen().is_some())}
/// Change the profile; the caller rebuilds every view that read the old one.
pub fn switch(profile:Graphics) {*GRAPHICS.lock().unwrap()=Some(profile);}
/// The Settings choice: "auto" (by chip) or a profile name.
pub fn chosen(choice:&str)->Graphics {Graphics::parse(choice).unwrap_or_else(|_|auto())}
fn initial()->Graphics {
    if let Some(p)=std::env::var("GG_GRAPHICS").ok().and_then(|s|Graphics::parse(&s).ok()) {return p;}
    if let Some(level)=app::graphics::chosen() {return Graphics::for_level(level);}
    saved_choice().map_or_else(auto,|c|chosen(&c))
}
/// `graphics_choice` from settings.json, read before the menus load it so
/// the first scene is built with it. Only a manual choice is saved; the
/// earlier `graphics` key also held automatic downgrades and is ignored.
fn saved_choice()->Option<String> {
    let text=std::fs::read_to_string(crate::play::settings_path()?).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?.get("graphics_choice")?.as_str().map(str::to_owned)
}
/// Without --graphics or GG_GRAPHICS: base Apple chips (M1, M2, M3, M4 without
/// Pro/Max/Ultra, 7-10 GPU cores) get LOW; full quality ran far too slow on an M1.
/// AUTO: this session's probe decision, else the newest probe for this chip
/// and build, else by chip.
pub fn auto()->Graphics {
    if let Some(p)=*PROBED.lock().unwrap() {return p;}
    if let Some(p)=cached_probe(None) {return p;}
    by_chip()
}
static PROBED:Mutex<Option<Graphics>>=Mutex::new(None);
/// The probe's decision for this session (AUTO resolves to it).
pub fn set_probed(profile:Graphics) {*PROBED.lock().unwrap()=Some(profile);}

/// The probe cache: per chip, build and output size, newest last.
fn cache_path()->Option<std::path::PathBuf> {Some(crate::play::settings_path()?.with_file_name("graphics-probe.json"))}
/// This build: the executable's size and modification time.
fn build_id()->String {
    std::env::current_exe().ok().and_then(|e|std::fs::metadata(e).ok()).map_or_else(String::new,|m|{
        let t=m.modified().ok().and_then(|t|t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0,|d|d.as_secs());
        format!("{}-{t}",m.len())})
}
fn cache_entries()->Vec<serde_json::Value> {
    cache_path().and_then(|p|std::fs::read_to_string(p).ok()).and_then(|t|serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v|v.get("entries")?.as_array().cloned()).unwrap_or_default()
}
/// The cached probe decision for this chip and build, for `size` (None: any
/// size, the newest; the window's size is not known before the scene).
pub fn cached_probe(size:Option<[u32;2]>)->Option<Graphics> {
    let (chip,build)=(chip_name().unwrap_or_default(),build_id());
    cache_entries().iter().rev().find(|e| e["chip"]==chip.as_str() && e["build"]==build.as_str()
        && size.is_none_or(|s|e["output"]==serde_json::json!(s)))
        .and_then(|e|Graphics::parse(e["profile"].as_str()?).ok())
}
/// Remember a probe decision (keeps the newest 8).
pub fn cache_probe(size:[u32;2],profile:Graphics,gpu_ms:f64) {
    let Some(path)=cache_path() else {return};
    let (chip,build)=(chip_name().unwrap_or_default(),build_id());
    let mut entries:Vec<_>=cache_entries().into_iter().filter(|e|!(e["chip"]==chip.as_str() && e["build"]==build.as_str() && e["output"]==serde_json::json!(size))).collect();
    entries.push(serde_json::json!({"chip":chip,"build":build,"output":size,"profile":profile.name(),"gpu_ms_median":gpu_ms}));
    let skip=entries.len().saturating_sub(8);
    if let Some(d)=path.parent() {let _=std::fs::create_dir_all(d);}
    let _=std::fs::write(path,serde_json::to_string_pretty(&serde_json::json!({"entries":entries[skip..]})).unwrap_or_default());
}

/// `quality.log` beside the settings (Application Support in the Mac app):
/// probe numbers and every adaptive decision, for players to send. Kept
/// under 256 KiB with one older file.
pub fn log(line:&str) {
    if cfg!(target_arch="wasm32") {return;}
    let dir=std::env::var_os("HOME").filter(|_|cfg!(target_os="macos")).map(|h|std::path::PathBuf::from(h).join("Library/Application Support/Growing Guns"))
        .or_else(||crate::play::settings_path()?.parent().map(std::path::Path::to_path_buf));
    let Some(dir)=dir else {return};
    let _=std::fs::create_dir_all(&dir);
    let path=dir.join("quality.log");
    if std::fs::metadata(&path).is_ok_and(|m|m.len()>256*1024) {let _=std::fs::rename(&path,dir.join("quality.log.1"));}
    let stamp=web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0,|d|d.as_secs());
    use std::io::Write;
    if let Ok(mut f)=std::fs::OpenOptions::new().create(true).append(true).open(path) {let _=writeln!(f,"{stamp} {line}");}
}

fn by_chip()->Graphics {
    static AUTO:OnceLock<Graphics>=OnceLock::new();
    *AUTO.get_or_init(||{
    let chip=chip_name();
    let profile=if chip.as_deref().is_some_and(base_apple_chip) {Graphics::LOW} else {Graphics::FULL};
    if let Some(chip)=chip {eprintln!("graphics: {chip} -> {}",profile.name());}
    profile
    })
}
#[cfg(target_os="macos")]
fn chip_name()->Option<String> {
    if let Some(chip)=app::emulate::chip_name() {return Some(chip);}
    let out=std::process::Command::new("/usr/sbin/sysctl").args(["-n","machdep.cpu.brand_string"]).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s|!s.is_empty())
}
#[cfg(not(target_os="macos"))]
fn chip_name()->Option<String> {app::emulate::chip_name()}
fn base_apple_chip(name:&str)->bool {
    let Some(rest)=name.strip_suffix(" (emulated)").unwrap_or(name).strip_prefix("Apple M") else {return false};
    rest.trim_start_matches(|c:char|c.is_ascii_digit()).trim().is_empty()
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn low_changes_only_visual_budgets() {
        assert!(!Graphics::LOW.stadium && !Graphics::LOW.sky_occlusion);
        assert_eq!(Graphics::LOW.debris_count(900),57);
        assert_eq!(Graphics::FULL.debris_count(900),900);
        assert!(Graphics::LOW.simple_blocks && Graphics::LOW.debris_size>1.0);
        assert!(Graphics::parse("typo").is_err());
        assert!(!Graphics::FULL.simple_guns && Graphics::LOW.simple_guns && Graphics::MINIMUM.simple_guns);
    }
    #[test] fn profiles_step_down_to_minimum_and_name_themselves() {
        assert_eq!(Graphics::FULL.lower(),Some(Graphics::LOW));
        assert_eq!(Graphics::LOW.lower(),Some(Graphics::MINIMUM));
        assert_eq!(Graphics::MINIMUM.lower(),None);
        for g in [Graphics::FULL,Graphics::LOW,Graphics::MINIMUM] {assert_eq!(Graphics::parse(g.name()),Ok(g));}
        assert_eq!(chosen("low"),Graphics::LOW);
        assert_eq!(chosen("auto"),auto());
    }
    #[test] fn only_base_apple_chips_default_to_low() {
        assert!(base_apple_chip("Apple M1") && base_apple_chip("Apple M2") && base_apple_chip("Apple M4"));
        assert!(!base_apple_chip("Apple M1 Pro") && !base_apple_chip("Apple M5 Pro") && !base_apple_chip("Apple M2 Max"));
        assert!(!base_apple_chip("Intel(R) Core(TM) i7") && !base_apple_chip("AMD Ryzen 9"));
    }
}
