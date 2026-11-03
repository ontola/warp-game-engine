//! Shared graphics modifiers: the simulation and collision do not change.
//! PC: --graphics full|low|minimum (or GG_GRAPHICS), else the Settings choice
//! (`graphics_choice` in settings.json), else by chip. Quest selects minimum. Without
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
    pub debris_capacity: usize,
    pub debris_divisor: u32,
    pub debris_size: f32,
    pub particles: usize,
    pub lod_pixels: f32,
}
impl Graphics {
    pub const FULL: Self=Self {minimal_forward:false,characters:true,stadium:true,sky_occlusion:true,simple_blocks:false,simple_guns:false,simple_lighting:false,debris_capacity:4096,debris_divisor:1,debris_size:1.0,particles:768,lod_pixels:1.0};
    pub const LOW: Self=Self {minimal_forward:false,characters:true,stadium:false,sky_occlusion:false,simple_blocks:true,simple_guns:true,simple_lighting:true,debris_capacity:128,debris_divisor:16,debris_size:1.6,particles:128,lod_pixels:4.0};
    pub const MINIMUM:Self=Self {minimal_forward:true,characters:false,debris_capacity:0,particles:32,lod_pixels:16.0,..Self::LOW};
    pub fn parse(s:&str)->Result<Self,String> {match s {"full"=>Ok(Self::FULL),"low"=>Ok(Self::LOW),"minimum"=>Ok(Self::MINIMUM),_=>Err(format!("unknown graphics profile {s}; use full, low or minimum"))}}
    pub fn debris_count(self,count:u32)->u32 {count.div_ceil(self.debris_divisor).min(self.debris_capacity as u32)}
    pub fn name(self)->&'static str {if self==Self::FULL {"full"} else if self==Self::LOW {"low"} else {"minimum"}}
    /// One step cheaper, if there is one.
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
pub fn fixed()->bool {*FIXED.get_or_init(||std::env::var("GG_GRAPHICS").is_ok_and(|s|Graphics::parse(&s).is_ok()))}
/// Change the profile; the caller rebuilds every view that read the old one.
pub fn switch(profile:Graphics) {*GRAPHICS.lock().unwrap()=Some(profile);}
/// The Settings choice: "auto" (by chip) or a profile name.
pub fn chosen(choice:&str)->Graphics {Graphics::parse(choice).unwrap_or_else(|_|auto())}
fn initial()->Graphics {
    if let Some(p)=std::env::var("GG_GRAPHICS").ok().and_then(|s|Graphics::parse(&s).ok()) {return p;}
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
pub fn auto()->Graphics {
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
    let out=std::process::Command::new("/usr/sbin/sysctl").args(["-n","machdep.cpu.brand_string"]).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s|!s.is_empty())
}
#[cfg(not(target_os="macos"))]
fn chip_name()->Option<String> {None}
fn base_apple_chip(name:&str)->bool {
    let Some(rest)=name.strip_prefix("Apple M") else {return false};
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
