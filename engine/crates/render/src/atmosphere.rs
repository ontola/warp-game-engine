//! Opt-in, bounded CPU atmosphere bake. No work is scheduled per frame/view.
//!
//! Rayleigh + aerosol Mie single scattering and ozone absorption, on a
//! spherical planet. A shared transmittance LUT accelerates sky integration;
//! the resulting sun-free radiance supplies BOTH the background and SH9.
//! This is a foundation, not Hillaire's complete multiple-scattering/aerial
//! perspective implementation. No clouds, stars, moon, or visible solar disc.
//!
//! Model references: https://sebh.github.io/publications/egsr2020.pdf and
//! https://github.com/sebh/UnrealEngineSkyAtmosphere (author's reference).
//! Independent implementation; quantities use kilometres and linear RGB.
use glam::{DVec3, Vec3};
use std::{f64::consts::PI, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atmosphere {
    pub planet_radius_km: f64,
    pub height_km: f64,
    pub observer_height_m: f64,
    pub rayleigh_per_km: DVec3,
    pub rayleigh_height_km: f64,
    pub mie_scattering_per_km: f64,
    pub mie_absorption_per_km: f64,
    pub mie_height_km: f64,
    pub mie_g: f64,
    pub ozone_per_km: DVec3,
    pub ground_albedo: DVec3,
    /// RGB illuminance on a sun-facing surface above the atmosphere.
    pub solar_rgb_lux: DVec3,
}
impl Default for Atmosphere {
    fn default() -> Self {
        Self { planet_radius_km:6360.0,height_km:100.0,observer_height_m:2.0,
            rayleigh_per_km:DVec3::new(0.005802,0.013558,0.033100),rayleigh_height_km:8.0,
            mie_scattering_per_km:0.003996,mie_absorption_per_km:0.000444,mie_height_km:1.2,mie_g:0.8,
            ozone_per_km:DVec3::new(0.000650,0.001881,0.000085),
            ground_albedo:DVec3::splat(0.15),solar_rgb_lux:DVec3::splat(127_500.0) }
    }
}
impl Atmosphere {
    pub fn validate(&self) -> Result<(),String> {
        let positive=[self.planet_radius_km,self.height_km,self.rayleigh_height_km,self.mie_height_km];
        let nonnegative=[self.observer_height_m,self.mie_scattering_per_km,self.mie_absorption_per_km];
        if !positive.iter().all(|x|x.is_finite()&&*x>0.0)
            || !nonnegative.iter().all(|x|x.is_finite()&&*x>=0.0)
            || self.planet_radius_km<100.0 || self.planet_radius_km>100_000.0
            || self.height_km>self.planet_radius_km*0.1 || self.height_km<1.0
            || self.observer_height_m>=self.height_km*1000.0
            || !self.mie_g.is_finite() || self.mie_g.abs()>=0.99
            || ![self.rayleigh_per_km,self.ozone_per_km,self.ground_albedo,self.solar_rgb_lux]
                .iter().all(|v|v.is_finite()&&v.min_element()>=0.0)
            || self.ground_albedo.max_element()>1.0
            || self.solar_rgb_lux.max_element()>1_000_000.0 {
            return Err("atmosphere requires finite, bounded radii, nonnegative coefficients, albedo0..1 and |Mie g|<0.99".into());
        }
        Ok(())
    }
    fn observer(&self)->DVec3 {DVec3::Y*(self.planet_radius_km+self.observer_height_m.max(0.01)*0.001)}
    fn medium(&self,height:f64)->(DVec3,DVec3,DVec3) {
        let h=height.max(0.0);
        let ray=self.rayleigh_per_km*(-h/self.rayleigh_height_km).exp();
        let mie=DVec3::splat(self.mie_scattering_per_km*(-h/self.mie_height_km).exp());
        let extinction=ray+mie+DVec3::splat(self.mie_absorption_per_km*(-h/self.mie_height_km).exp())
            +self.ozone_per_km*(1.0-(h-25.0).abs()/15.0).max(0.0);
        (ray,mie,extinction)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolarState { pub elevation_degrees:f64,pub azimuth_degrees:f64 }
impl SolarState {
    pub fn from_degrees(elevation:f64,azimuth:f64)->Result<Self,String> {
        if !elevation.is_finite() || !azimuth.is_finite() || !(-90.0..=90.0).contains(&elevation) {
            return Err("solar state needs finite elevation -90..90 and azimuth degrees".into());
        }
        Ok(Self {elevation_degrees:elevation,azimuth_degrees:azimuth.rem_euclid(360.0)})
    }
    pub fn direction(self)->DVec3 {
        let el=self.elevation_degrees.to_radians();let az=self.azimuth_degrees.to_radians();
        DVec3::new(az.sin()*el.cos(),el.sin(),-az.cos()*el.cos())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BakeQuality {
    pub transmittance_width:u32,pub transmittance_height:u32,pub transmittance_steps:u32,
    pub sky_width:u32,pub sky_height:u32,pub sky_steps:u32,
}
impl Default for BakeQuality {
    fn default()->Self {Self {transmittance_width:128,transmittance_height:64,transmittance_steps:96,
        sky_width:256,sky_height:128,sky_steps:48}}
}
impl BakeQuality {
    pub fn validate(self)->Result<(),String> {
        if !(8..=512).contains(&self.transmittance_width)||!(4..=256).contains(&self.transmittance_height)
            || !(8..=256).contains(&self.transmittance_steps)||!(16..=1024).contains(&self.sky_width)
            || !(8..=512).contains(&self.sky_height)||!(8..=128).contains(&self.sky_steps)
            || self.sky_width as u64*self.sky_height as u64*self.sky_steps as u64>8_388_608 {
            return Err("atmosphere bake exceeds explicit LUT/integration limits".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct BakedSky {
    pub width:u32,pub height:u32,
    /// Sun-disc-free linear radiance. Direction matches sky_dir_uv exactly.
    pub pixels:Vec<[f32;4]>,
    pub sun_dir:Vec3,pub sun_rgb_lux:Vec3,pub sky_horizontal_rgb_lux:Vec3,
    /// Raw radiance SH9 in the renderer's y/z/x convention (not convolved twice).
    pub sh:[[f32;4];9],pub horizon:Vec3,
    /// Local aerosol medium parameters, independent of the sky background.
    /// Density remains a presentation setting; these describe scattering.
    pub fog_phase_g:f32,pub fog_scattering_albedo:Vec3,pub fog_height_m:f32,
    pub quality:BakeQuality,pub solar:SolarState,
}
impl BakedSky {
    /// The same solar state drives direct light and the environment texture.
    /// This helper does not bake, allocate a scene, or change exposure/density.
    pub fn apply_lighting(&self,light:&mut crate::Lighting) {
        light.sun_dir=self.sun_dir;
        light.sun_lux=self.sun_rgb_lux.y.max(self.sun_rgb_lux.max_element()*1e-6);
        light.sun_color=if light.sun_lux>0.0 {self.sun_rgb_lux/light.sun_lux}else{Vec3::ONE};
        light.sky_lux=self.sky_horizontal_rgb_lux.y;
        light.fog_phase_g=self.fog_phase_g;
        light.fog_scattering_albedo=self.fog_scattering_albedo;
        light.fog_height_m=self.fog_height_m;
    }

    /// Reject malformed public bake data before changing any live GPU state.
    pub fn validate(&self)->Result<(),String> {
        self.quality.validate()?;
        let solar=SolarState::from_degrees(self.solar.elevation_degrees,self.solar.azimuth_degrees)?;
        if self.width!=self.quality.sky_width || self.height!=self.quality.sky_height
            || self.pixels.len()!=self.width as usize*self.height as usize
            || !self.pixels.iter().all(|p|p.iter().all(|v|v.is_finite()&&*v>=0.0))
            || !self.sun_dir.is_finite() || (self.sun_dir.length_squared()-1.0).abs()>1e-4
            || !self.sun_dir.abs_diff_eq(solar.direction().as_vec3(),1e-5)
            || ![self.sun_rgb_lux,self.sky_horizontal_rgb_lux,self.horizon].iter().all(|v|v.is_finite()&&v.min_element()>=0.0)
            || !self.fog_phase_g.is_finite() || self.fog_phase_g.abs()>=0.99
            || !self.fog_scattering_albedo.is_finite() || self.fog_scattering_albedo.min_element()<0.0 || self.fog_scattering_albedo.max_element()>1.0
            || !self.fog_height_m.is_finite() || self.fog_height_m<=0.0
            || !self.sh.iter().flatten().all(|v|v.is_finite()) {
            return Err("sky update requires matching dimensions and finite physical radiance/lighting".into());
        }
        Ok(())
    }

    /// Explicit opt-in. Call before creating the renderer; attach one result
    /// per shared world, never separately for each split-screen camera.
    pub fn apply(&self,scene:&mut crate::Scene,light:&mut crate::Lighting) {
        scene.sky=Some((self.width,self.height,self.pixels.clone()));
        scene.sky_physical=true;
        self.apply_lighting(light);
    }
}

#[derive(Default)]
pub struct AtmosphereCache {
    lut:Option<TransmittanceLut>,
    baked:Option<(Atmosphere,SolarState,BakeQuality,Arc<BakedSky>)>,
    pub transmittance_bakes:u64,pub sky_bakes:u64,
}
impl AtmosphereCache {
    /// Exact-key cache; no hidden time quantisation or per-frame scheduling.
    pub fn get_or_bake(&mut self,model:&Atmosphere,solar:SolarState,quality:BakeQuality)->Result<Arc<BakedSky>,String> {
        model.validate()?;quality.validate()?;
        let solar=SolarState::from_degrees(solar.elevation_degrees,solar.azimuth_degrees)?;
        if let Some((m,s,q,result))=&self.baked {if *m==*model&&*s==solar&&*q==quality {return Ok(result.clone());}}
        let key=(quality.transmittance_width,quality.transmittance_height,quality.transmittance_steps);
        if !self.lut.as_ref().is_some_and(|lut|lut.model==*model&&lut.key==key) {
            self.lut=Some(TransmittanceLut::new(*model,key));self.transmittance_bakes+=1;
        }
        let result=Arc::new(bake_sky(model,solar,quality,self.lut.as_ref().unwrap())?);
        self.sky_bakes+=1;self.baked=Some((*model,solar,quality,result.clone()));Ok(result)
    }
}

struct TransmittanceLut {model:Atmosphere,key:(u32,u32,u32),values:Vec<DVec3>}
fn top_distance(model:&Atmosphere,p:DVec3,d:DVec3)->f64 {
    let b=p.dot(d);let top=model.planet_radius_km+model.height_km;
    (-b+(b*b+top*top-p.length_squared()).max(0.0).sqrt()).max(0.0)
}
fn ground_distance(model:&Atmosphere,p:DVec3,d:DVec3)->Option<f64> {
    let b=p.dot(d);if b>=0.0 {return None;}
    let discriminant=b*b-(p.length_squared()-model.planet_radius_km.powi(2));
    if discriminant<0.0 {None}else{Some((-b-discriminant.sqrt()).max(0.0))}
}
fn exp_neg(v:DVec3)->DVec3 {DVec3::new((-v.x).exp(),(-v.y).exp(),(-v.z).exp())}
fn integrate_transmittance(model:&Atmosphere,p:DVec3,d:DVec3,steps:u32)->DVec3 {
    let distance=top_distance(model,p,d);let mut optical=DVec3::ZERO;
    for i in 0..steps {
        let a=(i as f64/steps as f64).powi(2)*distance;
        let b=((i+1)as f64/steps as f64).powi(2)*distance;
        let point=p+d*((a+b)*0.5);
        optical+=model.medium(point.length()-model.planet_radius_km).2*(b-a);
    }
    exp_neg(optical)
}
impl TransmittanceLut {
    fn new(model:Atmosphere,key:(u32,u32,u32))->Self {
        let (width,height,steps)=key;let mut values=Vec::with_capacity((width*height)as usize);
        let bottom=model.planet_radius_km;let top=bottom+model.height_km;
        let horizon=(top*top-bottom*bottom).sqrt();
        // Horizon-aware distance/radius coordinates from the atmosphere
        // literature: the table contains only rays that miss the planet.
        for y in 0..height {for x in 0..width {
            let rho=y as f64/(height-1)as f64*horizon;
            let radius=(bottom*bottom+rho*rho).sqrt();
            let minimum=top-radius;let maximum=rho+horizon;
            let distance=minimum+x as f64/(width-1)as f64*(maximum-minimum);
            let mu=if distance<=1e-10 {1.0}else{((top*top-radius*radius-distance*distance)/(2.0*radius*distance)).clamp(-1.0,1.0)};
            values.push(integrate_transmittance(&model,DVec3::Y*radius,DVec3::new((1.0-mu*mu).sqrt(),mu,0.0),steps));
        }}
        Self {model,key,values}
    }
    fn sample(&self,p:DVec3,d:DVec3)->DVec3 {
        if ground_distance(&self.model,p,d).is_some() {return DVec3::ZERO;}
        let bottom=self.model.planet_radius_km;let top=bottom+self.model.height_km;
        let radius=p.length().clamp(bottom,top);let horizon=(top*top-bottom*bottom).sqrt();
        let rho=(radius*radius-bottom*bottom).max(0.0).sqrt();
        let minimum=top-radius;let maximum=rho+horizon;
        let distance=top_distance(&self.model,p,d);
        let u=((distance-minimum)/(maximum-minimum).max(1e-10)).clamp(0.0,1.0);
        let v=(rho/horizon).clamp(0.0,1.0);
        let (w,h,_)=self.key;let x=u*(w-1)as f64;let y=v*(h-1)as f64;
        let x0=x.floor()as u32;let y0=y.floor()as u32;
        let get=|xx:u32,yy:u32|self.values[(yy.min(h-1)*w+xx.min(w-1))as usize];
        get(x0,y0).lerp(get(x0+1,y0),x.fract()).lerp(get(x0,y0+1).lerp(get(x0+1,y0+1),x.fract()),y.fract())
    }
}

fn rayleigh_phase(mu:f64)->f64 {3.0*(1.0+mu*mu)/(16.0*PI)}
fn mie_phase(mu:f64,g:f64)->f64 {
    // Normalised Cornette-Shanks aerosol phase; forward toward the sun.
    3.0*(1.0-g*g)*(1.0+mu*mu)/(8.0*PI*(2.0+g*g)*(1.0+g*g-2.0*g*mu).powf(1.5))
}
fn sky_direction(x:u32,y:u32,w:u32,h:u32)->DVec3 {
    let theta=(y as f64+0.5)/h as f64*PI;
    let phi=((x as f64+0.5)/w as f64-0.5)*2.0*PI;
    DVec3::new(phi.sin()*theta.sin(),theta.cos(),-phi.cos()*theta.sin())
}
fn trace_sky(model:&Atmosphere,lut:&TransmittanceLut,sun:DVec3,d:DVec3,steps:u32)->(DVec3,DVec3,bool) {
    let origin=model.observer();let ground=ground_distance(model,origin,d);
    let distance=ground.unwrap_or_else(||top_distance(model,origin,d));
    let mut transmittance=DVec3::ONE;let mut radiance=DVec3::ZERO;
    let phase_ray=rayleigh_phase(d.dot(sun));let phase_mie=mie_phase(d.dot(sun),model.mie_g);
    for i in 0..steps {
        let a=(i as f64/steps as f64).powi(2)*distance;
        let b=((i+1)as f64/steps as f64).powi(2)*distance;let ds=b-a;
        let p=origin+d*((a+b)*0.5);
        let (ray,mie,extinction)=model.medium(p.length()-model.planet_radius_km);
        let transmission=exp_neg(extinction*ds);
        let source=(ray*phase_ray+mie*phase_mie)*lut.sample(p,sun)*model.solar_rgb_lux;
        let segment=DVec3::new(
            if extinction.x>1e-12 {(1.0-transmission.x)/extinction.x}else{ds},
            if extinction.y>1e-12 {(1.0-transmission.y)/extinction.y}else{ds},
            if extinction.z>1e-12 {(1.0-transmission.z)/extinction.z}else{ds});
        radiance+=transmittance*source*segment;transmittance*=transmission;
    }
    (radiance,transmittance,ground.is_some())
}
fn horizontal_irradiance(w:u32,h:u32,pixels:&[[f32;4]])->DVec3 {
    let mut result=DVec3::ZERO;
    for y in 0..h {
        let t0=y as f64/h as f64*PI;let t1=((y+1)as f64/h as f64*PI).min(PI*0.5);
        if t0>=PI*0.5 {continue;}
        let weight=PI/w as f64*(t1.sin().powi(2)-t0.sin().powi(2));
        for x in 0..w {let p=pixels[(y*w+x)as usize];result+=DVec3::new(p[0]as f64,p[1]as f64,p[2]as f64)*weight;}
    }
    result
}
fn bake_sky(model:&Atmosphere,solar:SolarState,q:BakeQuality,lut:&TransmittanceLut)->Result<BakedSky,String> {
    let sun=solar.direction();let direct=lut.sample(model.observer(),sun)*model.solar_rgb_lux;
    let mut pixels=Vec::with_capacity((q.sky_width*q.sky_height)as usize);let mut ground_trans=Vec::with_capacity(pixels.capacity());
    for y in 0..q.sky_height {for x in 0..q.sky_width {
        let (radiance,transmittance,ground)=trace_sky(model,lut,sun,sky_direction(x,y,q.sky_width,q.sky_height),q.sky_steps);
        pixels.push([radiance.x as f32,radiance.y as f32,radiance.z as f32,1.0]);
        ground_trans.push(if ground {transmittance}else{DVec3::ZERO});
    }}
    let sky_horizontal=horizontal_irradiance(q.sky_width,q.sky_height,&pixels);
    // Lambertian lower hemisphere. This is one ground bounce only, not
    // atmospheric multiple scattering or terrain-aware aerial perspective.
    let ground=model.ground_albedo*(sky_horizontal+direct*sun.y.max(0.0))/PI;
    for (pixel,transmittance) in pixels.iter_mut().zip(ground_trans) {
        for channel in 0..3 {pixel[channel]+=(transmittance[channel]*ground[channel])as f32;}
    }
    if !pixels.iter().all(|p|p.iter().all(|v|v.is_finite()&&*v>=0.0)) {return Err("atmosphere produced invalid radiance".into());}
    let (sh,horizon)=crate::frame::sky_sh_small(q.sky_width,q.sky_height,&pixels,0.0,f32::MAX);
    let aerosol_extinction=model.mie_scattering_per_km+model.mie_absorption_per_km;
    let aerosol_albedo=if aerosol_extinction>0.0 {model.mie_scattering_per_km/aerosol_extinction}else{0.0};
    Ok(BakedSky {width:q.sky_width,height:q.sky_height,pixels,sun_dir:sun.as_vec3(),sun_rgb_lux:direct.as_vec3(),
        sky_horizontal_rgb_lux:sky_horizontal.as_vec3(),sh,horizon,quality:q,solar,
        fog_phase_g:model.mie_g as f32,fog_scattering_albedo:Vec3::splat(aerosol_albedo as f32),fog_height_m:(model.mie_height_km*1000.0)as f32})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn small()->BakeQuality {BakeQuality {transmittance_width:64,transmittance_height:32,transmittance_steps:64,
        sky_width:64,sky_height:32,sky_steps:32}}
    #[test]
    fn physical_terms_have_normalised_phase_and_bounded_transmittance() {
        let mut ray=0.0;let mut mie=0.0;let n=20_000;
        for i in 0..n {let mu=-1.0+(i as f64+0.5)*2.0/n as f64;ray+=rayleigh_phase(mu)*4.0*PI/n as f64;mie+=mie_phase(mu,0.8)*4.0*PI/n as f64;}
        assert!((ray-1.0).abs()<1e-7);assert!((mie-1.0).abs()<1e-5);
        let model=Atmosphere::default();let lut=TransmittanceLut::new(model,(128,64,96));
        for altitude in [0.01,1.0,12.0,80.0] {for elevation in [-20.0f64,0.0,1.0,10.0,80.0] {
            let p=DVec3::Y*(model.planet_radius_km+altitude);let d=SolarState::from_degrees(elevation,0.0).unwrap().direction();
            let actual=lut.sample(p,d);assert!(actual.is_finite()&&actual.min_element()>=0.0&&actual.max_element()<=1.0);
            if ground_distance(&model,p,d).is_some() {assert_eq!(actual,DVec3::ZERO);} else {
                let reference=integrate_transmittance(&model,p,d,512);
                assert!((actual-reference).abs().max_element()<0.035,"LUT vs independent high-step quadrature: {actual:?}/{reference:?}");
            }
        }}
    }
    #[test]
    fn sun_sky_and_sh_share_daylight_state_and_night_has_no_hidden_sun() {
        let model=Atmosphere::default();let mut cache=AtmosphereCache::default();
        let noon=cache.get_or_bake(&model,SolarState::from_degrees(60.0,125.0).unwrap(),small()).unwrap();
        let sunset=cache.get_or_bake(&model,SolarState::from_degrees(2.0,125.0).unwrap(),small()).unwrap();
        let night=cache.get_or_bake(&model,SolarState::from_degrees(-30.0,125.0).unwrap(),small()).unwrap();
        assert!(noon.sun_rgb_lux.y>50_000.0);assert!(sunset.sun_rgb_lux.y<noon.sun_rgb_lux.y);
        assert!(sunset.sun_rgb_lux.x/sunset.sun_rgb_lux.z>noon.sun_rgb_lux.x/noon.sun_rgb_lux.z);
        assert_eq!(night.sun_rgb_lux,Vec3::ZERO);assert!(night.sky_horizontal_rgb_lux.max_element()<0.001);
        for sky in [&noon,&sunset,&night] {
            sky.validate().unwrap();
            let (sh,horizon)=crate::frame::sky_sh_small(sky.width,sky.height,&sky.pixels,0.0,f32::MAX);
            assert_eq!(sh,sky.sh);assert_eq!(horizon,sky.horizon);
            let mut scene=crate::Scene::default();let mut light=crate::Lighting::default();sky.apply(&mut scene,&mut light);
            assert!(scene.sky_physical);assert_eq!(light.sun_dir,sky.sun_dir);
            assert!((light.sun_color*light.sun_lux-sky.sun_rgb_lux).abs().max_element()<0.02);
            assert_eq!(light.sky_lux,sky.sky_horizontal_rgb_lux.y);
        }
        assert_eq!(cache.transmittance_bakes,1);assert_eq!(cache.sky_bakes,3);
        let repeat=cache.get_or_bake(&model,night.solar,small()).unwrap();assert!(Arc::ptr_eq(&repeat,&night));assert_eq!(cache.sky_bakes,3);
    }

    #[test]
    fn live_bake_rejects_malformed_inputs_and_preserves_exposure_policy() {
        let sky=AtmosphereCache::default().get_or_bake(&Atmosphere::default(),
            SolarState::from_degrees(20.0,125.0).unwrap(),small()).unwrap();
        let mut light=crate::Lighting {ev100:12.25,fog_density:0.012,..Default::default()};
        sky.apply_lighting(&mut light);
        assert_eq!(light.ev100,12.25);assert_eq!(light.fog_density,0.012);
        assert_eq!(light.fog_phase_g,sky.fog_phase_g);
        assert_eq!(light.fog_scattering_albedo,sky.fog_scattering_albedo);
        assert_eq!(light.fog_height_m,sky.fog_height_m);
        for case in 0..8 {
            let mut invalid=(*sky).clone();
            match case {
                0=>{invalid.pixels.pop();},
                1=>invalid.pixels[0][0]=f32::NAN,
                2=>invalid.sun_dir=Vec3::ZERO,
                3=>invalid.sun_rgb_lux.x=-1.0,
                4=>invalid.sh[0][0]=f32::INFINITY,
                5=>invalid.fog_phase_g=1.0,
                6=>invalid.fog_scattering_albedo.x=1.01,
                _=>invalid.fog_height_m=f32::NAN,
            }
            assert!(invalid.validate().is_err(),"case {case}");
        }
    }
    #[test]
    fn local_aerosol_optics_follow_model_instead_of_sky_pixels() {
        let mut cache=AtmosphereCache::default();
        for (scatter,absorb,height,g,expected) in [(0.003996,0.000444,1.2,0.8,0.9),(0.0,0.004,0.22,0.2,0.0),(0.0,0.0,0.7,0.0,0.0)] {
            let model=Atmosphere {mie_scattering_per_km:scatter,mie_absorption_per_km:absorb,mie_height_km:height,mie_g:g,..Default::default()};
            let sky=cache.get_or_bake(&model,SolarState::from_degrees(2.0,270.0).unwrap(),small()).unwrap();
            assert!((sky.fog_scattering_albedo.x-expected).abs()<1e-6);
            assert_eq!(sky.fog_height_m,(height*1000.0)as f32);
            assert_eq!(sky.fog_phase_g,g as f32);
            sky.validate().unwrap();
        }
    }
    #[test]
    fn vacuum_has_no_diffuse_sun_double_count_and_invalid_models_fail() {
        let vacuum=Atmosphere {rayleigh_per_km:DVec3::ZERO,mie_scattering_per_km:0.0,mie_absorption_per_km:0.0,
            ozone_per_km:DVec3::ZERO,ground_albedo:DVec3::ZERO,..Default::default()};
        let mut cache=AtmosphereCache::default();let sun=SolarState::from_degrees(30.0,0.0).unwrap();
        let sky=cache.get_or_bake(&vacuum,sun,small()).unwrap();
        assert_eq!(sky.sun_rgb_lux,vacuum.solar_rgb_lux.as_vec3());assert_eq!(sky.sky_horizontal_rgb_lux,Vec3::ZERO);
        assert!(sky.sh.iter().flatten().all(|&v|v==0.0));
        for model in [Atmosphere {mie_g:1.0,..vacuum},Atmosphere {observer_height_m:f64::NAN,..vacuum},Atmosphere {ground_albedo:DVec3::splat(-0.1),..vacuum}] {
            assert!(cache.get_or_bake(&model,sun,small()).is_err());
        }
        assert!(SolarState::from_degrees(100.0,0.0).is_err());assert!(SolarState::from_degrees(0.0,f64::NAN).is_err());
        assert!(cache.get_or_bake(&vacuum,sun,BakeQuality {sky_width:100_000,..small()}).is_err());
    }
}
