//! Live procedural sky: time-of-day and weather presets over
//! [`crate::atmosphere`], with a latest-wins background bake worker.
//!
//! A game picks a [`LiveSettings`] (a [`DayTime`] preset and a
//! [`WeatherLook`], or a custom sun), bakes it once with [`bake_live`] before
//! the renderer exists, or requests it from a [`LiveSky`] while running. The
//! worker bakes off the frame loop; a finished [`PreparedSky`] goes to
//! `Renderer::update_sky` plus [`PreparedSky::apply_scene`]. Nothing here
//! bakes per frame or per camera. See engine/docs/live-sky-controls.md.
use crate::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, BakedSky, SolarState};
use crate::{Lighting, Scene};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// Provenance of the artistic night environment in a bake, for metadata.
pub fn night_metadata(sky:&BakedSky)->serde_json::Value {
    serde_json::json!({"configured_ambient_rgb_lux":sky.night.ambient_rgb_lux.to_array(),
        "configured_star_radiance":sky.night.star_radiance,"ambient_rgb_lux":sky.night_ambient_rgb_lux.to_array(),
        "star_peak_radiance":sky.night_stars[0],"star_zenith_optical_depth":sky.night_stars[1],
        "star_seed":sky.night.star_seed,"twilight_elevation_degrees":[-12.0,-6.0],
        "mode":"artistic sky floor plus stable background points; not bounce GI or a moon model"})
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
    /// The preset's exposure (EV100).
    pub fn exposure(self)->f32 {match self {Self::Sunrise|Self::Sunset=>11.0,Self::Morning|Self::Afternoon=>13.0,Self::Noon=>14.0,Self::Night=>6.0}}
    pub fn step(self,delta:i8)->Self {let n=Self::ALL.iter().position(|v|*v==self).unwrap() as i32;Self::ALL[(n+delta as i32).rem_euclid(6) as usize]}
}

/// Exposure (EV100) for any solar elevation, piecewise linear through the
/// [`DayTime`] presets (night -12 deg EV6, sunrise/sunset 2 deg EV11,
/// morning/afternoon ~26 deg EV13, noon 60 deg EV14). For continuous cycles.
pub fn exposure_for_elevation(elevation_degrees:f64)->f32 {
    const KNOTS:[(f64,f64);4]=[(-12.0,6.0),(2.0,11.0),(26.0,13.0),(60.0,14.0)];
    let e=elevation_degrees.clamp(KNOTS[0].0,KNOTS[3].0);
    let k=KNOTS.windows(2).find(|w|e<=w[1].0).unwrap();
    (k[0].1+(k[1].1-k[0].1)*(e-k[0].0)/(k[1].0-k[0].0)) as f32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherLook { Clear, Haze, Mist }
impl WeatherLook {
    pub const ALL:[Self;3]=[Self::Clear,Self::Haze,Self::Mist];
    pub fn key(self)->&'static str {match self {Self::Clear=>"clear",Self::Haze=>"haze",Self::Mist=>"mist"}}
    pub fn label(self)->&'static str {match self {Self::Clear=>"Clear",Self::Haze=>"Haze",Self::Mist=>"Mist"}}
    pub fn parse(s:&str)->Result<Self,String> {Self::ALL.into_iter().find(|v|v.key()==s).ok_or_else(||format!("unknown atmosphere: {s}"))}
    pub fn step(self,delta:i8)->Self {let n=Self::ALL.iter().position(|v|*v==self).unwrap() as i32;Self::ALL[(n+delta as i32).rem_euclid(3) as usize]}
    /// The atmosphere this look bakes (aerosol scale and height).
    pub fn model(self)->Atmosphere {
        let mut model=Atmosphere::default();
        let (scale,height)=match self {Self::Clear=>(1.0,1.2),Self::Haze=>(4.0,1.2),Self::Mist=>(14.0,0.22)};
        model.mie_scattering_per_km*=scale;model.mie_absorption_per_km*=scale;model.mie_height_km=height;model
    }
    /// Distance fog: start m, end m, density per m.
    pub fn fog(self)->(f32,f32,f32) {match self {Self::Clear=>(180.0,1200.0,0.0),Self::Haze=>(90.0,650.0,0.0012),Self::Mist=>(20.0,190.0,0.011)}}
}

/// A time preset and weather look, optionally with a custom sun and exposure
/// (which then win over the preset's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveSettings {pub time:DayTime,pub atmosphere:WeatherLook,solar_override:Option<SolarState>,exposure_override:Option<f32>}
impl Default for LiveSettings {fn default()->Self {Self {time:DayTime::Noon,atmosphere:WeatherLook::Clear,solar_override:None,exposure_override:None}}}
impl LiveSettings {
    pub fn preset(time:DayTime,atmosphere:WeatherLook)->Self {Self {time,atmosphere,..Default::default()}}
    /// A custom sun and exposure (a cycle or a recorded light); the time
    /// preset stays for labels and is reported as "current".
    pub fn with_sun(self,solar:SolarState,ev100:f32)->Self {Self {solar_override:Some(solar),exposure_override:Some(ev100),..self}}
    /// Keep the sun (preset or custom) and set only the exposure, e.g. a
    /// game whose own lamps or emissive surfaces set the night exposure.
    pub fn with_exposure(self,ev100:f32)->Self {Self {exposure_override:Some(ev100),..self}}
    pub fn changed(self,key:&str,value:&serde_json::Value)->Result<Self,String> {
        let value=value.as_str().ok_or("live sky setting must be a string")?;
        match key {"sky_time" if value=="current"=>Ok(self),
            "sky_time"=>Ok(Self {time:DayTime::parse(value)?,solar_override:None,exposure_override:None,..self}),
            "sky_weather"=>Ok(Self {atmosphere:WeatherLook::parse(value)?,..self}),_=>Err("unknown live sky setting".into())}
    }
    pub fn time_key(self)->&'static str {if self.solar_override.is_some(){"current"}else{self.time.key()}}
    pub fn time_label(self)->&'static str {if self.solar_override.is_some(){"Current"}else{self.time.label()}}
    pub fn solar(self)->SolarState {self.solar_override.unwrap_or_else(||self.time.solar())}
    pub fn exposure(self)->f32 {self.exposure_override.unwrap_or_else(||self.time.exposure())}
    pub fn metadata(self)->serde_json::Value {serde_json::json!({"time_of_day":self.time_key(),"atmosphere":self.atmosphere.key(),"solar_elevation_degrees":self.solar().elevation_degrees,"solar_azimuth_degrees":self.solar().azimuth_degrees,"ev100":self.exposure()})}
    /// Fog, exposure and shadow strength; the bake supplies sun and sky.
    pub fn apply_presentation(self,light:&mut Lighting) {
        let (start,end,density)=self.atmosphere.fog();light.fog_start=start;light.fog_end=end;light.fog_density=density;
        light.ev100=self.exposure();light.shadow_strength=1.0;
    }
}

#[derive(Clone)]
pub struct PreparedSky {pub revision:u64,pub settings:LiveSettings,pub sky:Arc<BakedSky>,pub metadata:serde_json::Value}
impl PreparedSky {
    /// The sky texture, sun, skylight, fog and exposure, together. For a
    /// live renderer call `Renderer::update_sky` with the same light too.
    pub fn apply_scene(&self,scene:&mut Scene,light:&mut Lighting) {
        self.sky.apply(scene,light);self.settings.apply_presentation(light);
    }
}

/// Bake (or reuse from `cache`) one sky synchronously. Metadata records the
/// light, fog, LUT size, cache reuse and the bake's CPU time.
pub fn bake_live(revision:u64,settings:LiveSettings,cache:&mut AtmosphereCache,quality:BakeQuality)->Result<PreparedSky,String> {
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
        std::thread::Builder::new().name("sky-bake".into()).spawn(move||{
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

/// Live sky changes: a lazy background worker (started on the first
/// request), a 150 ms debounce, at most one bake in flight, the newest
/// request waiting, stale results discarded, the last eight skies reused.
pub struct LiveSky {schedule:Schedule,worker:Option<Worker>,applied:Option<PreparedSky>,error:Option<String>,initial:serde_json::Value}
impl LiveSky {
    /// From the sky a world started with: its metadata and light. A sun that
    /// matches no preset becomes a custom ("current") sun.
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
    /// Starting from known settings (already applied, e.g. by [`bake_live`]).
    pub fn starting(settings:LiveSettings,initial:&serde_json::Value)->Self {
        let mut schedule=Schedule::new();schedule.desired=settings;
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
    /// Call every frame: dispatches due work and returns the newest finished
    /// sky, if any. Then `accepted` (or `failed` if the GPU update failed).
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
    pub fn busy(&self)->bool {self.schedule.busy()}
    pub fn status(&self)->&str {
        if self.error.is_some() {"Sky update failed; current sky kept"}else if self.schedule.busy(){"Updating sky..."}else if self.applied.is_some(){"Applied"}else{"Adjust to apply a live sky preset"}
    }
    pub fn metadata(&self)->serde_json::Value {serde_json::json!({"desired":self.schedule.desired.metadata(),"requested_revision":self.schedule.revision,
        "pending":self.schedule.pending.is_some(),"in_flight_revision":self.schedule.in_flight,
        "applied":self.applied.as_ref().map(|s|serde_json::json!({"revision":s.revision,"settings":s.settings.metadata(),"sky":s.metadata})),"initial_sky":self.initial,"error":self.error})}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small()->BakeQuality {BakeQuality {sky_width:32,sky_height:16,sky_steps:16,transmittance_width:32,transmittance_height:16,transmittance_steps:32}}

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
        let quality=small();
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
        let mut cache=AtmosphereCache::default();
        let night=bake_live(1,LiveSettings {time:DayTime::Night,..Default::default()},&mut cache,small()).unwrap();
        let mut scene=Scene::default();scene.aux=vec![7,8,9];
        let mut light=Lighting::default();night.apply_scene(&mut scene,&mut light);
        assert_eq!(scene.aux,vec![7,8,9]);assert_eq!(light.ev100,6.0);
        // This intentionally coarse 16-row LUT has <0.02 lux quadrature
        // error at the configured 10 green-channel lux; exposure stays EV6.
        assert_eq!(light.sun_lux,0.0);assert!((light.sky_lux-10.0).abs()<0.02);
        assert_eq!(light.night_stars[0],180.0);
        assert_eq!(night.metadata["night_environment"]["configured_ambient_rgb_lux"],serde_json::json!([8.0,10.0,12.0]));
        assert_eq!(night.metadata["night_environment"]["star_seed"],0x5eed);
        let day=bake_live(2,LiveSettings::default(),&mut cache,small()).unwrap();
        day.apply_scene(&mut scene,&mut light);
        assert_eq!(light.night_stars[0],0.0,"a live time change must remove stars in daylight");
        assert_eq!(day.sky.night_ambient_rgb_lux,glam::Vec3::ZERO);
    }

    #[test]
    fn exposure_curve_meets_every_preset_and_custom_sun_wins() {
        for t in [DayTime::Sunrise,DayTime::Sunset,DayTime::Noon,DayTime::Night] {
            assert_eq!(exposure_for_elevation(t.solar().elevation_degrees),t.exposure());
        }
        assert!((exposure_for_elevation(24.0)-DayTime::Morning.exposure()).abs()<0.2);
        assert_eq!(exposure_for_elevation(-90.0),6.0);assert_eq!(exposure_for_elevation(90.0),14.0);
        let sun=SolarState::from_degrees(10.0,200.0).unwrap();
        let s=LiveSettings::preset(DayTime::Noon,WeatherLook::Haze).with_sun(sun,12.0);
        assert_eq!((s.solar(),s.exposure(),s.time_key()),(sun,12.0,"current"));
        let live=LiveSky::starting(s,&serde_json::Value::Null);assert_eq!(live.desired(),s);assert!(!live.busy());
        let lit=LiveSettings::preset(DayTime::Night,WeatherLook::Clear).with_exposure(11.0);
        assert_eq!((lit.solar(),lit.exposure(),lit.time_key()),(DayTime::Night.solar(),11.0,"night"));
    }

    /// Bake cost per preset at the default quality (release: `cargo test
    /// --release -p render --lib sky::tests::bake_cost -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn bake_cost() {
        for atmosphere in WeatherLook::ALL {
            let mut cache=AtmosphereCache::default();
            for time in DayTime::ALL {
                let t=Instant::now();bake_live(0,LiveSettings::preset(time,atmosphere),&mut cache,BakeQuality::default()).unwrap();
                println!("bake {} {}: {:.1} ms (transmittance bakes {})",time.key(),atmosphere.key(),t.elapsed().as_secs_f64()*1000.0,cache.transmittance_bakes);
            }
        }
    }
}
