//! Explicit developer opt-in for the atmosphere foundation. A single cached
//! world sky feeds all views; this never runs inside the frame loop.
use render::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, SolarState};
use render::{Lighting, Scene};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Config {
    solar: SolarState,
}

fn night_metadata(sky:&render::atmosphere::BakedSky)->serde_json::Value {
    serde_json::json!({"configured_ambient_rgb_lux":sky.night.ambient_rgb_lux.to_array(),
        "configured_star_radiance":sky.night.star_radiance,"ambient_rgb_lux":sky.night_ambient_rgb_lux.to_array(),
        "star_peak_radiance":sky.night_stars[0],"star_zenith_optical_depth":sky.night_stars[1],
        "star_seed":sky.night.star_seed,"twilight_elevation_degrees":[-12.0,-6.0],
        "mode":"artistic sky floor plus stable background points; not bounce GI or a moon model"})
}

fn parse(mut get: impl FnMut(&str) -> Option<String>) -> Result<Option<Config>, String> {
    match get("SIDEWAYS_SKY").as_deref() {
        None | Some("hdri") => return Ok(None),
        Some("procedural") => {},
        Some(_) => return Err("SIDEWAYS_SKY must be hdri or procedural".into()),
    }
    let mut number = |name, default| -> Result<f64, String> {
        get(name).map(|value| value.parse::<f64>().map_err(|_| format!("{name} must be a finite number")))
            .transpose().map(|value| value.unwrap_or(default))
    };
    let elevation = number("SIDEWAYS_SUN_ELEVATION", 60.0)?;
    let azimuth = number("SIDEWAYS_SUN_AZIMUTH", 125.0)?;
    Ok(Some(Config { solar: SolarState::from_degrees(elevation, azimuth)? }))
}

/// Apply before Renderer::new, once for the world. Stage rebuilds with the
/// same physical parameters reuse the bake. `--set` light overrides in a shot
/// may deliberately run afterwards; metadata records both the bake and light.
pub fn apply_environment(scene: &mut Scene, light: &mut Lighting) -> Result<serde_json::Value, String> {
    let Some(config) = parse(|name| std::env::var(name).ok())? else {
        return Ok(serde_json::json!({"mode":"hdri"}));
    };
    static CACHE: OnceLock<Mutex<AtmosphereCache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(|| Mutex::new(AtmosphereCache::default())).lock()
        .map_err(|_| "procedural sky cache lock poisoned".to_string())?;
    apply(config, &mut cache, BakeQuality::default(), scene, light)
}

fn apply(config: Config, cache: &mut AtmosphereCache, quality: BakeQuality, scene: &mut Scene, light: &mut Lighting)
    -> Result<serde_json::Value, String>
{
    let before = cache.sky_bakes;
    let sky = cache.get_or_bake(&Atmosphere::default(), config.solar, quality)?;
    sky.apply(scene, light);
    let reused = cache.sky_bakes == before;
    eprintln!("procedural sky: elevation {} deg, azimuth {} deg, cached {reused}; single scattering, no clouds or live time progression",
        config.solar.elevation_degrees, config.solar.azimuth_degrees);
    Ok(serde_json::json!({
        "mode":"procedural_single_scattering", "cached":reused,
        "solar_elevation_degrees":config.solar.elevation_degrees,
        "solar_azimuth_degrees":config.solar.azimuth_degrees,
        "sun_rgb_lux":sky.sun_rgb_lux.to_array(),
        "sky_horizontal_rgb_lux":sky.sky_horizontal_rgb_lux.to_array(),
        "night_environment":night_metadata(&sky),
        "sky_lut":[quality.sky_width,quality.sky_height],
        "transmittance_lut":[quality.transmittance_width,quality.transmittance_height],
        "fog_medium":{"height_m":sky.fog_height_m,"phase_g":sky.fog_phase_g,
            "scattering_albedo":sky.fog_scattering_albedo.to_array(),"density_per_m":light.fog_density,
            "mode":"local_exponential_height_single_scattering","hard_end":false},
        "limitations":["no volumetric clouds or cloud shadows", "no atmospheric multiple scattering",
            "local height aerosol: camera-terrain sun gate; no along-ray scene occlusion or aerial perspective LUT",
            "no visible solar disc, moon or automatic time progression",
            "stars use angular point filtering, not an astronomical catalogue; no stellar reflections or integrated stellar GI"]
    }))
}

/// Artistic time presets, not a geographic/astronomical calendar model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DayTime { Sunrise, Morning, Noon, Afternoon, Sunset, Night }
impl DayTime {
    pub const ALL:[Self;6]=[Self::Sunrise,Self::Morning,Self::Noon,Self::Afternoon,Self::Sunset,Self::Night];
    pub fn key(self)->&'static str {match self {Self::Sunrise=>"sunrise",Self::Morning=>"morning",Self::Noon=>"noon",Self::Afternoon=>"afternoon",Self::Sunset=>"sunset",Self::Night=>"night"}}
    pub fn label(self)->&'static str {match self {Self::Sunrise=>"Sunrise",Self::Morning=>"Morning",Self::Noon=>"Noon",Self::Afternoon=>"Afternoon",Self::Sunset=>"Sunset",Self::Night=>"Night"}}
    pub fn parse(s:&str)->Result<Self,String> {Self::ALL.into_iter().find(|v|v.key()==s).ok_or_else(||format!("unknown time of day: {s}"))}
    pub fn solar(self)->SolarState {
        let (el,az)=match self {Self::Sunrise=>(2.0,75.0),Self::Morning=>(24.0,105.0),Self::Noon=>(60.0,125.0),Self::Afternoon=>(28.0,235.0),Self::Sunset=>(2.0,270.0),Self::Night=>(-12.0,320.0)};
        SolarState::from_degrees(el,az).unwrap()
    }
    fn exposure(self)->f32 {match self {Self::Sunrise|Self::Sunset=>11.0,Self::Morning|Self::Afternoon=>13.0,Self::Noon=>14.0,Self::Night=>6.0}}
    pub fn step(self,delta:i8)->Self {let n=Self::ALL.iter().position(|v|*v==self).unwrap() as i32;Self::ALL[(n+delta as i32).rem_euclid(6) as usize]}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherLook { Clear, Haze, Mist }
impl WeatherLook {
    pub const ALL:[Self;3]=[Self::Clear,Self::Haze,Self::Mist];
    pub fn key(self)->&'static str {match self {Self::Clear=>"clear",Self::Haze=>"haze",Self::Mist=>"mist"}}
    pub fn label(self)->&'static str {match self {Self::Clear=>"Clear",Self::Haze=>"Haze",Self::Mist=>"Mist"}}
    pub fn parse(s:&str)->Result<Self,String> {Self::ALL.into_iter().find(|v|v.key()==s).ok_or_else(||format!("unknown atmosphere: {s}"))}
    pub fn step(self,delta:i8)->Self {let n=Self::ALL.iter().position(|v|*v==self).unwrap() as i32;Self::ALL[(n+delta as i32).rem_euclid(3) as usize]}
    fn model(self)->Atmosphere {
        let mut model=Atmosphere::default();
        let (scale,height)=match self {Self::Clear=>(1.0,1.2),Self::Haze=>(4.0,1.2),Self::Mist=>(14.0,0.22)};
        model.mie_scattering_per_km*=scale;model.mie_absorption_per_km*=scale;model.mie_height_km=height;model
    }
    fn fog(self)->(f32,f32,f32) {match self {Self::Clear=>(180.0,1200.0,0.0),Self::Haze=>(90.0,650.0,0.0012),Self::Mist=>(20.0,190.0,0.011)}}
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveSettings {pub time:DayTime,pub atmosphere:WeatherLook,solar_override:Option<SolarState>,exposure_override:Option<f32>}
impl Default for LiveSettings {fn default()->Self {Self {time:DayTime::Noon,atmosphere:WeatherLook::Clear,solar_override:None,exposure_override:None}}}
impl LiveSettings {
    pub fn changed(self,key:&str,value:&serde_json::Value)->Result<Self,String> {
        let value=value.as_str().ok_or("live sky setting must be a string")?;
        match key {"sky_time" if value=="current"=>Ok(self),
            "sky_time"=>Ok(Self {time:DayTime::parse(value)?,solar_override:None,exposure_override:None,..self}),
            "sky_weather"=>Ok(Self {atmosphere:WeatherLook::parse(value)?,..self}),_=>Err("unknown live sky setting".into())}
    }
    pub fn time_key(self)->&'static str {if self.solar_override.is_some(){"current"}else{self.time.key()}}
    pub fn time_label(self)->&'static str {if self.solar_override.is_some(){"Current"}else{self.time.label()}}
    fn solar(self)->SolarState {self.solar_override.unwrap_or_else(||self.time.solar())}
    fn exposure(self)->f32 {self.exposure_override.unwrap_or_else(||self.time.exposure())}
    fn metadata(self)->serde_json::Value {serde_json::json!({"time_of_day":self.time_key(),"atmosphere":self.atmosphere.key(),"solar_elevation_degrees":self.solar().elevation_degrees,"solar_azimuth_degrees":self.solar().azimuth_degrees,"ev100":self.exposure()})}
    pub fn apply_presentation(self,light:&mut Lighting) {
        let (start,end,density)=self.atmosphere.fog();light.fog_start=start;light.fog_end=end;light.fog_density=density;
        light.ev100=self.exposure();light.shadow_strength=1.0;
    }
}

#[derive(Clone)]
pub struct PreparedSky {pub revision:u64,pub settings:LiveSettings,pub sky:Arc<render::atmosphere::BakedSky>,pub metadata:serde_json::Value}
impl PreparedSky {
    pub fn apply_scene(&self,scene:&mut Scene,light:&mut Lighting) {
        self.sky.apply(scene,light);self.settings.apply_presentation(light);
    }
}
pub(crate) fn bake_live(revision:u64,settings:LiveSettings,cache:&mut AtmosphereCache,quality:BakeQuality)->Result<PreparedSky,String> {
    let started=Instant::now();let before=cache.sky_bakes;
    let sky=cache.get_or_bake(&settings.atmosphere.model(),settings.solar(),quality)?;
    let metadata=serde_json::json!({"mode":"procedural_single_scattering","time_of_day":settings.time_key(),"atmosphere":settings.atmosphere.key(),
        "solar_elevation_degrees":sky.solar.elevation_degrees,"solar_azimuth_degrees":sky.solar.azimuth_degrees,
        "sun_rgb_lux":sky.sun_rgb_lux.to_array(),"sky_horizontal_rgb_lux":sky.sky_horizontal_rgb_lux.to_array(),
        "night_environment":night_metadata(&sky),
        "sky_lut":[sky.width,sky.height],"cached":cache.sky_bakes==before,"bake_cpu_ms":started.elapsed().as_secs_f64()*1000.0,
        "ev100":settings.exposure(),"fog_start_end_density":settings.atmosphere.fog(),
        "fog_medium":{"height_m":sky.fog_height_m,"phase_g":sky.fog_phase_g,
            "scattering_albedo":sky.fog_scattering_albedo.to_array(),"density_per_m":settings.atmosphere.fog().2,
            "mode":"local_exponential_height_single_scattering","hard_end":false},
        "limits":["single scattering","no clouds, rain or moon","stars use angular point filtering, not an astronomical catalogue; no stellar reflections or integrated stellar GI","local height aerosol: camera-terrain sun gate; no along-ray scene occlusion or aerial perspective LUT","road grip unchanged"]});
    Ok(PreparedSky {revision,settings,sky,metadata})
}

/// Latest-wins scheduler, with one bounded in-flight job and one pending value.
/// A stale completion must never replace a newer requested time/weather pair.
struct Schedule {revision:u64,desired:LiveSettings,pending:Option<Instant>,in_flight:Option<u64>}
impl Schedule {
    fn new()->Self {Self {revision:0,desired:LiveSettings::default(),pending:None,in_flight:None}}
    fn request(&mut self,settings:LiveSettings,now:Instant) {
        self.revision=self.revision.wrapping_add(1);self.desired=settings;self.pending=Some(now+Duration::from_millis(150));
    }
    fn dispatch(&mut self,now:Instant)->Option<(u64,LiveSettings)> {
        if self.in_flight.is_some() || !self.pending.is_some_and(|due|now>=due) {return None;}
        self.pending=None;self.in_flight=Some(self.revision);Some((self.revision,self.desired))
    }
    fn complete(&mut self,revision:u64)->bool {
        if self.in_flight==Some(revision) {self.in_flight=None;}
        revision==self.revision
    }
    fn busy(&self)->bool {self.pending.is_some()||self.in_flight.is_some()}
}

struct Worker {send:mpsc::SyncSender<(u64,LiveSettings)>,receive:mpsc::Receiver<(u64,Result<PreparedSky,String>)>}
impl Worker {
    fn new()->Result<Self,String> {
        let (send,jobs)=mpsc::sync_channel::<(u64,LiveSettings)>(1);let (results,receive)=mpsc::sync_channel(1);
        std::thread::Builder::new().name("sideways-sky-bake".into()).spawn(move||{
            let mut cache=AtmosphereCache::default();let mut recent:std::collections::VecDeque<PreparedSky>=Default::default();
            while let Ok((revision,settings))=jobs.recv() {
                let output=if let Some(index)=recent.iter().position(|v|v.settings==settings) {
                    let mut value=recent.remove(index).unwrap();value.revision=revision;value.metadata["cached"]=true.into();value.metadata["bake_cpu_ms"]=0.0.into();
                    recent.push_back(value.clone());Ok(value)
                } else {
                    bake_live(revision,settings,&mut cache,BakeQuality::default()).inspect(|value|{
                        recent.push_back(value.clone());while recent.len()>8 {recent.pop_front();}
                    })
                };
                if results.send((revision,output)).is_err() {break;}
            }
        }).map_err(|e|format!("cannot start sky worker: {e}"))?;
        Ok(Self {send,receive})
    }
}

pub struct LiveSky {schedule:Schedule,worker:Option<Worker>,applied:Option<PreparedSky>,error:Option<String>,initial:serde_json::Value}
impl LiveSky {
    pub fn new(initial:&serde_json::Value,light:&Lighting)->Self {
        let mut schedule=Schedule::new();let direction=light.sun_dir.normalize_or_zero();
        let elevation=initial["solar_elevation_degrees"].as_f64().unwrap_or((direction.y as f64).asin().to_degrees());
        let azimuth=initial["solar_azimuth_degrees"].as_f64().unwrap_or((direction.x as f64).atan2(-direction.z as f64).to_degrees());
        let solar=SolarState::from_degrees(elevation,azimuth).unwrap_or_else(|_|DayTime::Noon.solar());
        if let Some(time)=DayTime::ALL.into_iter().find(|t|(t.solar().elevation_degrees-solar.elevation_degrees).abs()<1e-4&&(t.solar().azimuth_degrees-solar.azimuth_degrees).abs()<1e-4) {schedule.desired.time=time;}
        else {schedule.desired.solar_override=Some(solar);}
        schedule.desired.exposure_override=Some(light.ev100);
        Self {schedule,worker:None,applied:None,error:None,initial:initial.clone()}
    }
    pub fn desired(&self)->LiveSettings {self.schedule.desired}
    /// Immutable applied sky for a background scene build. Pending requests
    /// are applied to the finished scene later by the normal main-thread poll.
    pub fn applied_snapshot(&self)->Option<PreparedSky> {self.applied.clone()}
    pub fn request(&mut self,settings:LiveSettings,now:Instant)->Result<(),String> {
        if self.worker.is_none() {self.worker=Some(Worker::new()?);}
        self.error=None;self.schedule.request(settings,now);Ok(())
    }
    pub fn poll(&mut self,now:Instant)->Option<PreparedSky> {
        let mut ready=None;
        let mut disconnected=false;
        if let Some(worker)=&self.worker {
            match worker.receive.try_recv() {
                Ok((revision,result))=>if self.schedule.complete(revision) {match result {Ok(sky)=>ready=Some(sky),Err(e)=>self.error=Some(e)}},
                Err(mpsc::TryRecvError::Disconnected)=>{self.schedule.in_flight=None;self.schedule.pending=None;self.error=Some("sky worker disconnected".into());disconnected=true;},
                Err(mpsc::TryRecvError::Empty)=>{},
            }
            if let Some(job)=self.schedule.dispatch(now) {
                if let Err(e)=worker.send.try_send(job) {self.schedule.complete(job.0);self.error=Some(format!("sky worker stopped: {e}"));}
            }
        }
        if disconnected {self.worker=None;}
        ready
    }
    pub fn accepted(&mut self,sky:PreparedSky) {self.error=None;self.applied=Some(sky);}
    pub fn failed(&mut self,error:String) {self.error=Some(error);}
    pub fn restore(&self,scene:&mut Scene,light:&mut Lighting)->Option<serde_json::Value> {
        self.applied.as_ref().map(|sky|{sky.apply_scene(scene,light);sky.metadata.clone()})
    }
    pub fn status(&self)->&str {
        if self.error.is_some() {"Sky update failed; current sky kept"}else if self.schedule.busy(){"Updating sky..."}else if self.applied.is_some(){"Applied"}else{"Adjust to apply a live sky preset"}
    }
    pub fn metadata(&self)->serde_json::Value {serde_json::json!({"desired":self.schedule.desired.metadata(),"requested_revision":self.schedule.revision,
        "pending":self.schedule.pending.is_some(),"in_flight_revision":self.schedule.in_flight,
        "applied":self.applied.as_ref().map(|s|serde_json::json!({"revision":s.revision,"settings":s.settings.metadata(),"sky":s.metadata})),"initial_sky":self.initial,"error":self.error})}
}

/// Pure menu navigation state. One shared menu, never one per viewport.
#[derive(Default)]
pub struct PauseMenu {pub selected:usize,axis:[i8;2],next_repeat:Option<Instant>,confirm_held:bool}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum MenuAction {Resume,NewStage,Time(i8),Atmosphere(i8)}
impl PauseMenu {
    pub fn opened(&mut self,axis:[i8;2],confirm:bool) {self.selected=0;self.axis=axis;self.confirm_held=confirm;self.next_repeat=None;}
    /// Consume held input while an overlay owns focus, retaining selection.
    pub fn suspend(&mut self,axis:[i8;2],confirm:bool) {self.axis=axis;self.confirm_held=confirm;self.next_repeat=None;}
    pub fn navigate(&mut self,axis:[i8;2],confirm:bool,now:Instant)->Option<MenuAction> {
        let fire=axis!=[0,0]&&(axis!=self.axis||self.next_repeat.is_some_and(|time|now>=time));
        let confirm_edge=confirm&&!self.confirm_held;self.confirm_held=confirm;
        if fire {
            self.next_repeat=Some(now+Duration::from_millis(if axis==self.axis {200}else{380}));
            if axis[1]!=0 {self.selected=(self.selected as i32+axis[1] as i32).rem_euclid(4) as usize;}
        } else if axis==[0,0] {self.next_repeat=None;}
        self.axis=axis;
        let delta=if fire {axis[0]}else{0};
        match self.selected {
            0 if confirm_edge=>Some(MenuAction::Resume),3 if confirm_edge=>Some(MenuAction::NewStage),
            1 if delta!=0||confirm_edge=>Some(MenuAction::Time(if delta!=0 {delta}else{1})),
            2 if delta!=0||confirm_edge=>Some(MenuAction::Atmosphere(if delta!=0 {delta}else{1})),_=>None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_suspension_preserves_menu_selection_and_consumes_held_input() {
        let now=Instant::now();let mut menu=PauseMenu::default();menu.selected=1;
        assert_eq!(menu.navigate([1,0],false,now),Some(MenuAction::Time(1)));
        menu.suspend([1,0],true);
        assert_eq!(menu.navigate([1,0],true,now+Duration::from_secs(5)),None);
        assert_eq!(menu.selected,1);
        assert_eq!(menu.navigate([0,0],false,now+Duration::from_secs(6)),None);
        assert_eq!(menu.navigate([0,0],true,now+Duration::from_secs(7)),Some(MenuAction::Time(1)));
    }

    fn config(entries: &[(&str, &str)]) -> Result<Option<Config>, String> {
        parse(|name| entries.iter().find(|(key, _)| *key == name).map(|(_, value)| value.to_string()))
    }

    #[test]
    fn defaults_preserve_hdri_and_explicit_solar_state_is_validated() {
        assert_eq!(config(&[]).unwrap(), None);
        assert_eq!(config(&[("SIDEWAYS_SKY", "hdri")]).unwrap(), None);
        let noon = config(&[("SIDEWAYS_SKY", "procedural")]).unwrap().unwrap();
        assert_eq!(noon.solar, SolarState::from_degrees(60.0, 125.0).unwrap());
        let sunset = config(&[("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_ELEVATION", "-3"), ("SIDEWAYS_SUN_AZIMUTH", "-15")]).unwrap().unwrap();
        assert_eq!(sunset.solar, SolarState::from_degrees(-3.0, 345.0).unwrap());
        for entries in [vec![("SIDEWAYS_SKY", "clouds")],
            vec![("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_ELEVATION", "NaN")],
            vec![("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_ELEVATION", "91")],
            vec![("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_AZIMUTH", "infinity")],
            vec![("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_AZIMUTH", "bad")]] {
            assert!(config(&entries).is_err());
        }
    }

    #[test]
    fn metadata_describes_applied_light_and_repeated_world_reuses_bake() {
        let config = Config { solar: SolarState::from_degrees(12.0, 345.0).unwrap() };
        let quality = BakeQuality { sky_width:32, sky_height:16, sky_steps:16,
            transmittance_width:32, transmittance_height:16, transmittance_steps:32 };
        let mut cache = AtmosphereCache::default();
        let mut first_scene = Scene::default();
        let mut first_light = Lighting::default();
        let first = apply(config, &mut cache, quality, &mut first_scene, &mut first_light).unwrap();
        let mut second_scene = Scene::default();
        let mut second_light = Lighting::default();
        let second = apply(config, &mut cache, quality, &mut second_scene, &mut second_light).unwrap();
        assert_eq!(first["cached"], false);
        assert_eq!(second["cached"], true);
        assert_eq!(first["solar_elevation_degrees"], 12.0);
        assert_eq!(first["solar_azimuth_degrees"], 345.0);
        assert_eq!(first["sky_lut"], serde_json::json!([32,16]));
        let recorded: Vec<f32> = serde_json::from_value(first["sun_rgb_lux"].clone()).unwrap();
        let actual = first_light.sun_color * first_light.sun_lux;
        for channel in 0..3 { assert!((recorded[channel] - actual[channel]).abs() < 0.02); }
        assert!(first_scene.sky_physical && second_scene.sky_physical);
        assert_eq!(first_scene.sky, second_scene.sky);
        assert_eq!(first_light.sun_lux, second_light.sun_lux);
        assert_eq!(cache.sky_bakes, 1);
        assert_eq!(cache.transmittance_bakes, 1);
    }

    #[test]
    fn live_requests_coalesce_debounce_and_never_apply_stale_results() {
        let now=Instant::now();let mut schedule=Schedule::new();
        let morning=LiveSettings {time:DayTime::Morning,..Default::default()};
        let sunset=LiveSettings {time:DayTime::Sunset,..Default::default()};
        let mist=LiveSettings {atmosphere:WeatherLook::Mist,..sunset};
        schedule.request(morning,now);schedule.request(sunset,now+Duration::from_millis(10));
        assert_eq!(schedule.dispatch(now+Duration::from_millis(150)),None);
        let (first,settings)=schedule.dispatch(now+Duration::from_millis(160)).unwrap();assert_eq!(settings,sunset);
        schedule.request(mist,now+Duration::from_millis(200));
        assert_eq!(schedule.dispatch(now+Duration::from_secs(1)),None,"one in-flight bake only");
        assert!(!schedule.complete(first),"stale sunset may not flash before newer mist");
        let (last,settings)=schedule.dispatch(now+Duration::from_secs(1)).unwrap();assert_eq!(settings,mist);
        assert!(schedule.complete(last));assert!(!schedule.busy());assert_eq!(schedule.dispatch(now+Duration::from_secs(2)),None);
    }

    #[test]
    fn initial_custom_sun_is_preserved_by_weather_only_change() {
        let light=Lighting {sun_dir:SolarState::from_degrees(7.0,211.0).unwrap().direction().as_vec3(),ev100:12.3,..Default::default()};
        let controls=LiveSky::new(&serde_json::json!({"mode":"hdri"}),&light);
        assert_eq!(controls.desired().time_label(),"Current");
        let changed=controls.desired().changed("sky_weather",&"mist".into()).unwrap();
        assert!(changed.solar().direction().as_vec3().abs_diff_eq(light.sun_dir,1e-6));assert_eq!(changed.exposure(),12.3);
        let noon=changed.changed("sky_time",&"noon".into()).unwrap();assert_eq!(noon.solar(),DayTime::Noon.solar());
        assert_eq!(noon.exposure(),14.0);
    }

    #[test]
    fn live_settings_change_sun_sky_and_fog_together_without_scene_geometry_reset() {
        let quality=BakeQuality {sky_width:32,sky_height:16,sky_steps:16,transmittance_width:32,transmittance_height:16,transmittance_steps:32};
        let mut cache=AtmosphereCache::default();let noon=LiveSettings::default();
        let sunset=noon.changed("sky_time",&"sunset".into()).unwrap();
        let mist=sunset.changed("sky_weather",&"mist".into()).unwrap();
        assert_eq!(mist.time,DayTime::Sunset);assert!(noon.changed("sky_weather",&"rain".into()).is_err());
        let clear=bake_live(1,noon,&mut cache,quality).unwrap();let changed=bake_live(2,mist,&mut cache,quality).unwrap();
        let mut scene=Scene::default();scene.aux=vec![42,9,125];
        let mut light=Lighting::default();clear.apply_scene(&mut scene,&mut light);let initial_sun=light.sun_dir;
        changed.apply_scene(&mut scene,&mut light);
        assert_eq!(scene.aux,vec![42,9,125]);assert!(scene.sky_physical);
        assert_eq!(light.sun_dir,changed.sky.sun_dir);assert_ne!(light.sun_dir,initial_sun);
        assert_eq!(light.fog_density,WeatherLook::Mist.fog().2);assert_eq!(light.ev100,DayTime::Sunset.exposure());
        assert_eq!(light.sky_lux,changed.sky.sky_horizontal_rgb_lux.y);
        let actual=light.sun_color*light.sun_lux;assert!(actual.abs_diff_eq(changed.sky.sun_rgb_lux,0.01));
        assert_ne!(clear.sky.pixels,changed.sky.pixels);
        for look in WeatherLook::ALL {look.model().validate().unwrap();}
    }

    #[test]
    fn night_preset_applies_readable_sky_and_stars_without_exposure_or_geometry_hacks() {
        let quality=BakeQuality {sky_width:32,sky_height:16,sky_steps:16,transmittance_width:32,
            transmittance_height:16,transmittance_steps:32};
        let mut cache=AtmosphereCache::default();
        let night=bake_live(1,LiveSettings {time:DayTime::Night,..Default::default()},&mut cache,quality).unwrap();
        let mut scene=Scene::default();scene.aux=vec![7,8,9];
        let mut light=Lighting::default();night.apply_scene(&mut scene,&mut light);
        assert_eq!(scene.aux,vec![7,8,9]);assert_eq!(light.ev100,6.0);
        // This intentionally coarse 16-row LUT has <0.02 lux quadrature
        // error at the configured 10 green-channel lux; exposure stays EV6.
        assert_eq!(light.sun_lux,0.0);assert!((light.sky_lux-10.0).abs()<0.02);
        assert_eq!(light.night_stars[0],180.0);
        assert_eq!(night.metadata["night_environment"]["configured_ambient_rgb_lux"],serde_json::json!([8.0,10.0,12.0]));
        assert_eq!(night.metadata["night_environment"]["star_seed"],0x5eed);
        let day=bake_live(2,LiveSettings::default(),&mut cache,quality).unwrap();
        day.apply_scene(&mut scene,&mut light);
        assert_eq!(light.night_stars[0],0.0,"a live time change must remove stars in daylight");
        assert_eq!(day.sky.night_ambient_rgb_lux,glam::Vec3::ZERO);
    }

    #[test]
    fn pause_menu_edges_repeat_and_start_release_are_bounded() {
        let now=Instant::now();let mut menu=PauseMenu::default();
        menu.opened([0,0],true);assert_eq!(menu.navigate([0,0],true,now),None,"held handbrake must not instantly resume");
        menu.navigate([0,0],false,now);
        assert_eq!(menu.navigate([0,1],false,now),None);assert_eq!(menu.selected,1);
        assert_eq!(menu.navigate([1,0],false,now),Some(MenuAction::Time(1)));
        assert_eq!(menu.navigate([1,0],false,now+Duration::from_millis(100)),None);
        assert_eq!(menu.navigate([1,0],false,now+Duration::from_millis(380)),Some(MenuAction::Time(1)));
        menu.navigate([0,1],false,now+Duration::from_secs(1));assert_eq!(menu.selected,2);
        assert_eq!(menu.navigate([-1,0],false,now+Duration::from_secs(1)),Some(MenuAction::Atmosphere(-1)));
        assert_eq!(DayTime::Sunrise.step(-1),DayTime::Night);assert_eq!(WeatherLook::Clear.step(-1),WeatherLook::Mist);
    }
}
