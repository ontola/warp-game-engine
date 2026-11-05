//! Opt-in colour-geometry ablations. Removing an occluder changes work behind
//! it: measured differences are marginal/non-additive, never category timings.
use crate::scene::{GeometryCategory, GpuInstance, Scene};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Config {
    pub enabled: bool,
    pub excluded_mask: u32,
    pub near_grass_present: bool,
    pub far_grass_present: bool,
    pub transparency_present: bool,
}
impl Config {
    pub fn from_environment()->Result<Self,String> {match std::env::var("KERN_DIAGNOSTIC_HIDE") {
        Ok(value)=>Self::parse(Some(&value)),Err(std::env::VarError::NotPresent)=>Self::parse(None),
        Err(_)=>Err("KERN_DIAGNOSTIC_HIDE must be valid UTF-8 category names".into()),
    }}
    pub fn excluded_names(&self)->Vec<&'static str> {GeometryCategory::ALL.into_iter().filter(|c|self.excludes(*c)).map(|c|c.name()).collect()}
    pub fn excludes(&self,category:GeometryCategory)->bool {self.excluded_mask & category.exclusion_bit()!=0}
    pub fn grass_enabled(&self)->bool {!self.excludes(GeometryCategory::Grass)}
    fn validate_experimental_foliage(&self,present:bool)->Result<(),String> {
        if self.excluded_mask!=0 && present {
            return Err("category ablations do not support experimental voxel/part foliage; disable that explicit experiment before measuring".into());
        }
        Ok(())
    }
    fn parse(value:Option<&str>)->Result<Self,String> {
        let mut config=Self {enabled:value.is_some(),excluded_mask:0,near_grass_present:false,far_grass_present:false,transparency_present:false};
        if let Some(value)=value {
            if value=="none" {return Ok(config);}
            if value.is_empty() {return Err("KERN_DIAGNOSTIC_HIDE must be 'none' or comma-separated other,terrain,trees,grass,cars,structures".into());}
            for name in value.split(',').map(str::trim) {
                let category=GeometryCategory::ALL.into_iter().find(|category|category.name()==name)
                    .ok_or_else(||format!("unknown KERN_DIAGNOSTIC_HIDE category {name:?}"))?;
                config.excluded_mask|=category.exclusion_bit();
            }
        }
        Ok(config)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Counts {
    pub category: GeometryCategory,
    /// Distinct owners in the selected list; no empty reserved slots counted.
    pub instances: u64,
    pub clusters: u64,
    pub triangles: u64,
    /// Actual padded bucket vertex count, not triangles multiplied by three.
    pub submitted_vertices: u64,
}
#[derive(Debug)]
pub struct PixelCoverage {
    pub frame_id:u64,
    pub render_sequence:u64,
    pub view_index:usize,
    pub input_size:[u32;2],
    /// Main visibility pixels, including opaque grass; excludes transition/far
    /// grass, blended glass, effects, reconstruction and HUD.
    pub pixels:[u64;5],
    pub uncovered:u64,
}

/// Source-instance ownership in the opaque INPUT visibility buffer. This is
/// not final composited ownership: volume foliage, transition/far grass,
/// blended glass, effects, reconstruction and HUD are outside this mask.
#[derive(Debug)]
pub struct OwnerPixels {
    pub frame_id:u64,
    pub render_sequence:u64,
    pub view_index:usize,
    pub input_size:[u32;2],
    /// Row-major source instance indices; u32::MAX denotes uncovered pixels.
    pub owners:Vec<u32>,
}

pub(crate) struct State {
    config:Result<Config,String>,
    owners:Mutex<Option<Arc<Vec<GeometryCategory>>>>,
    triangles:Vec<u32>,
}
impl State {
    pub fn new(scene:&Scene)->Self {
        let mut config=Config::from_environment();
        if let Ok(c)=&mut config {
            c.near_grass_present=!scene.grass_ranges.is_empty();
            c.far_grass_present=scene.far_grass.is_some();
            c.transparency_present=!scene.blended_materials.is_empty();
            if let Err(error)=c.validate_experimental_foliage(scene.voxel_forest.is_some() || !scene.part_wind.is_empty() || !scene.foliage_representations.is_empty()) {
                config=Err(error);
            }
        }
        let enabled=config.as_ref().is_ok_and(|c|c.enabled);
        Self {config,owners:Mutex::new(enabled.then(||Arc::new(scene.instances.iter().map(GpuInstance::category).collect()))),
            triangles:if enabled {scene.clusters.iter().map(|c|c.tri_count).collect()}else{Vec::new()}}
    }
    pub fn config(&self)->Result<Config,String>{self.config.clone()}
    pub fn owners(&self)->Option<Arc<Vec<GeometryCategory>>>{self.owners.lock().unwrap().clone()}
    pub fn update(&self,first:usize,instances:&[GpuInstance]) {
        let mut owners=self.owners.lock().unwrap();
        if let Some(owners)=owners.as_mut() {
            // Flags normally stay unchanged, including occupied/empty grass
            // slots. Only an actual relabel copies a retained frame snapshot.
            if owners[first..first+instances.len()].iter().zip(instances).any(|(&old,new)|old!=new.category()) {
                for (old,new) in Arc::make_mut(owners)[first..first+instances.len()].iter_mut().zip(instances) {*old=new.category();}
            }
        }
    }
    pub fn count(&self,pairs:&[[u32;2]],owners:&[GeometryCategory],bucketed:bool)->Result<[Counts;6],String> {
        aggregate(pairs,owners,&self.triangles,bucketed)
    }
    pub fn owner_pixels(&self,raw:&[u8],size:[u32;2],pairs:&[[u32;2]],owner_count:usize)->Result<Vec<u32>,String> {
        decode_owner_pixels(raw,size,pairs,owner_count,&self.triangles)
    }
}

fn decode_owner_pixels(raw:&[u8],size:[u32;2],pairs:&[[u32;2]],owner_count:usize,triangles:&[u32])->Result<Vec<u32>,String> {
    let pixel_count=(size[0]as usize).checked_mul(size[1]as usize).ok_or("visibility dimensions overflow")?;
    let byte_count=pixel_count.checked_mul(4).ok_or("visibility byte count overflow")?;
    if size.contains(&0)||raw.len()!=byte_count {return Err("visibility readback length does not match input dimensions".into());}
    raw.chunks_exact(4).map(|value| {
        let id=u32::from_le_bytes(value.try_into().unwrap());
        if id==u32::MAX {return Ok(u32::MAX);}
        let &[owner,cluster]=pairs.get((id>>7)as usize).ok_or("visibility pixel refers to absent selected cluster")?;
        if owner as usize>=owner_count {return Err("visibility pixel owner out of bounds".into());}
        let count=*triangles.get(cluster as usize).ok_or("visibility pixel cluster out of bounds")?;
        if count>128||(id&127)>=count {return Err("visibility pixel triangle out of bounds".into());}
        Ok(owner)
    }).collect()
}

pub(crate) fn aggregate(pairs:&[[u32;2]],owners:&[GeometryCategory],triangles:&[u32],bucketed:bool)->Result<[Counts;6],String> {
    let mut result=GeometryCategory::ALL.map(|category|Counts{category,instances:0,clusters:0,triangles:0,submitted_vertices:0});
    let mut selected=std::collections::HashSet::new();
    for &[owner,cluster] in pairs {
        let category=*owners.get(owner as usize).ok_or("diagnostic selected owner out of bounds")?;
        let triangles=*triangles.get(cluster as usize).ok_or("diagnostic selected cluster out of bounds")?;
        if triangles>128 {return Err("diagnostic cluster exceeds raster bucket capacity".into());}
        let row=&mut result[category as usize];
        row.instances+=u64::from(selected.insert(owner));row.clusters+=1;row.triangles+=u64::from(triangles);
        row.submitted_vertices+=if !bucketed {384}else{match triangles {0..=8=>24,9..=32=>96,33..=64=>192,_=>384}};
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn category_names_are_strict_and_baseline_is_explicit() {
        assert!(!Config::parse(None).unwrap().enabled);
        let base=Config::parse(Some("none")).unwrap();assert!(base.enabled);assert_eq!(base.excluded_mask,0);
        let hide=Config::parse(Some("trees, grass,cars")).unwrap();assert_eq!(hide.excluded_names(),["trees","grass","cars"]);
        for wrong in ["","tree","none,cars","terrain,"] {assert!(Config::parse(Some(wrong)).is_err());}
    }
    #[test]
    fn experimental_foliage_allows_read_only_baseline_but_not_category_removal() {
        assert!(Config::parse(Some("none")).unwrap().validate_experimental_foliage(true).is_ok());
        assert!(Config::parse(None).unwrap().validate_experimental_foliage(true).is_ok());
        for name in ["grass","trees","terrain","cars","other"] {
            let config=Config::parse(Some(name)).unwrap();
            assert!(config.validate_experimental_foliage(true).is_err());
            assert!(config.validate_experimental_foliage(false).is_ok());
        }
    }
    #[test]
    fn visibility_owner_decode_preserves_background_and_selected_order() {
        let raw:Vec<u8>=[u32::MAX,128+3,1,128].into_iter().flat_map(u32::to_le_bytes).collect();
        assert_eq!(decode_owner_pixels(&raw,[2,2],&[[7,1],[2,0]],8,&[4,2]).unwrap(),[u32::MAX,2,7,2]);
        assert_eq!(decode_owner_pixels(&u32::MAX.to_le_bytes(),[1,1],&[],0,&[]).unwrap(),[u32::MAX]);
    }
    #[test]
    fn visibility_owner_decode_rejects_invalid_pixels_and_readback_shape() {
        let decode=|id:u32,pairs:&[[u32;2]],owners,triangles:&[u32]|decode_owner_pixels(&id.to_le_bytes(),[1,1],pairs,owners,triangles);
        assert!(decode(128,&[[0,0]],1,&[1]).is_err());
        assert!(decode(0,&[[1,0]],1,&[1]).is_err());
        assert!(decode(0,&[[0,1]],1,&[1]).is_err());
        assert!(decode(1,&[[0,0]],1,&[1]).is_err());
        assert!(decode(0,&[[0,0]],1,&[0]).is_err());
        assert!(decode(0,&[[0,0]],1,&[129]).is_err());
        assert!(decode_owner_pixels(&[0;3],[1,1],&[[0,0]],1,&[1]).is_err());
        assert!(decode_owner_pixels(&[0;4],[2,1],&[[0,0]],1,&[1]).is_err());
        assert!(decode_owner_pixels(&[],[0,1],&[],0,&[]).is_err());
    }
    #[test]
    fn selected_work_counts_unique_owners_and_actual_bucket_padding() {
        use GeometryCategory::*;
        let owners=[Trees,Grass,Terrain,Cars,Other];let triangles=[6,8,9,32,33,64,65,128];
        let pairs=[[0,0],[0,1],[1,2],[2,3],[3,4],[3,5],[4,6],[4,7]];
        let a=aggregate(&pairs,&owners,&triangles,true).unwrap();
        assert_eq!((a[2].instances,a[2].clusters,a[2].triangles,a[2].submitted_vertices),(1,2,14,48));
        assert_eq!(a.iter().map(|a|a.submitted_vertices).sum::<u64>(),1392);
        assert_eq!(a.iter().map(|a|a.instances).sum::<u64>(),5);
        let flat=aggregate(&pairs,&owners,&triangles,false).unwrap();assert_eq!(flat.iter().map(|a|a.submitted_vertices).sum::<u64>(),8*384);
        assert!(aggregate(&[[9,0]],&owners,&triangles,true).is_err());assert!(aggregate(&[[0,20]],&owners,&triangles,true).is_err());
    }
    #[test]
    fn owner_snapshots_survive_relabel_and_unchanged_uploads_do_not_copy() {
        use bytemuck::Zeroable;
        let state=State {config:Config::parse(Some("none")),owners:Mutex::new(Some(Arc::new(vec![GeometryCategory::Grass]))),triangles:Vec::new()};
        let old=state.owners().unwrap();let mut instance=GpuInstance::zeroed();instance.set_category(GeometryCategory::Grass);
        state.update(0,&[instance]);assert!(Arc::ptr_eq(&old,&state.owners().unwrap()));
        instance.set_category(GeometryCategory::Cars);state.update(0,&[instance]);
        assert_eq!(old[0],GeometryCategory::Grass);assert_eq!(state.owners().unwrap()[0],GeometryCategory::Cars);
    }
}
