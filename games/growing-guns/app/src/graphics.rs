//! Shared startup graphics modifiers: the simulation and collision do not change.
//! PC: --graphics full|low|minimum (or GG_GRAPHICS). Quest selects minimum.
use std::sync::OnceLock;
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
}
static GRAPHICS:OnceLock<Graphics>=OnceLock::new();
pub fn configure(profile:Graphics)->Result<(),String> {
    if let Some(current)=GRAPHICS.get() {return if *current==profile {Ok(())}else{Err("graphics profile is fixed when the scene is created; restart to change it".into())};}
    GRAPHICS.set(profile).map_err(|_|"graphics profile already configured".into())
}
pub fn get()->Graphics {*GRAPHICS.get_or_init(||std::env::var("GG_GRAPHICS").ok().and_then(|s|Graphics::parse(&s).ok()).unwrap_or_else(auto))}
/// Without --graphics or GG_GRAPHICS: base Apple chips (M1, M2, M3, M4 without
/// Pro/Max/Ultra, 7-10 GPU cores) get LOW; full quality ran far too slow on an M1.
fn auto()->Graphics {
    let chip=chip_name();
    let profile=if chip.as_deref().is_some_and(base_apple_chip) {Graphics::LOW} else {Graphics::FULL};
    if let Some(chip)=chip {eprintln!("graphics: {chip} -> {}",if profile==Graphics::LOW {"low"} else {"full"});}
    profile
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
    #[test] fn only_base_apple_chips_default_to_low() {
        assert!(base_apple_chip("Apple M1") && base_apple_chip("Apple M2") && base_apple_chip("Apple M4"));
        assert!(!base_apple_chip("Apple M1 Pro") && !base_apple_chip("Apple M5 Pro") && !base_apple_chip("Apple M2 Max"));
        assert!(!base_apple_chip("Intel(R) Core(TM) i7") && !base_apple_chip("AMD Ryzen 9"));
    }
}
