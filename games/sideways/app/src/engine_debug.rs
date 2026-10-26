//! Sideways adapter for Warp's reusable diagnostics widgets and timing model.
//! Live edits are process-local; explicit tree Apply rebuilds the stage's
//! rendering/collision resources. Reset uses the saved Sideways preset.
use render::{debug_settings::DebugRenderOptions, debug_ui::{DebugUi, DebugModel, DebugControl, DebugMenu, DebugValue, DebugPass, DebugKey, DebugAction},
    debug_timing::{TimingModel, CpuFrameTiming}, gpu_profile::GpuProfileDrain, hud::HudDrawList, Lighting};
use std::{collections::VecDeque, time::{Duration, Instant}};
use serde_json::{json, Value};

const GRASS:u32=1; const HEADLIGHTS:u32=2; const GLASS:u32=3; const EXHAUST:u32=4;
const PARTICLES:u32=5; const DISTANCE:u32=6; const LOD:u32=7; const EXPOSURE:u32=8; const FOG:u32=9;
const GRASS_SIZE:u32=10; const GRASS_COUNT:u32=11; const GRASS_RED:u32=12; const GRASS_GREEN:u32=13; const GRASS_BLUE:u32=14;
const GRASS_LOD:u32=15;
const TREE_SIZE:u32=20; const TREE_COUNT:u32=21; const TREE_RED:u32=22; const TREE_GREEN:u32=23; const TREE_BLUE:u32=24;
const TREE_APPLY:u32=25; const TREE_DISCARD:u32=26;
const BLOOM:u32=38;const BLOOM_STRENGTH:u32=39;
const VEGETATION_MENU:u32=100; const GRASS_MENU:u32=101; const TREES_MENU:u32=102; const ADVANCED_MENU:u32=103;

enum ScriptEvent { Key(DebugKey), Pointer([f32;2]), Down, Up }

pub struct EngineDebug {
    pub ui:DebugUi,
    pub options:DebugRenderOptions,
    baseline:crate::render_baseline::Preset,
    pub gpu_unavailable:bool,
    timing:TimingModel,
    model:DebugModel,
    model_at:Option<Instant>,
    cpu_start:Option<Instant>,
    cpu_execution:Option<render::cpu_time::ThreadCpuTimer>,
    cutoff_frame:u64,
    current_frame:u64,
    lod_pixels:f32,
    exposure_offset:f32,
    fog:bool,
    script:VecDeque<(u64,ScriptEvent)>,
    changes:u64,
    history_dirty:bool,
    pub tree_pending:crate::tree_tuning::Settings,
    tree_applied:crate::tree_tuning::Settings,
    tree_apply_requested:bool,
    tree_status:String,
    pub tree_report:Value,
    environment:crate::debug_environment::EnvironmentDebug,
}
impl EngineDebug {
    pub fn new(open:bool,script:Option<&str>)->Result<Self,String> {
        Self::with_baseline(open,script,crate::render_baseline::Preset::load()?)
    }
    fn with_baseline(open:bool,script:Option<&str>,baseline:crate::render_baseline::Preset)->Result<Self,String> {
        let mut ui=DebugUi::default();ui.open=open;
        Ok(Self {ui,options:baseline.grass,baseline,gpu_unavailable:false,timing:TimingModel::new(32),
            model:Default::default(),model_at:None,cpu_start:None,cpu_execution:None,cutoff_frame:0,current_frame:0,
            lod_pixels:1.0,exposure_offset:0.0,fog:true,
            script:if let Some(path)=script {load_script(path)?}else{VecDeque::new()},changes:0,history_dirty:false,
            tree_pending:baseline.trees,tree_applied:baseline.trees,tree_apply_requested:false,
            tree_status:"Current stage. Apply rebuilds trees, shadows and collisions; car position is preserved.".into(),tree_report:Value::Null,
            environment:crate::debug_environment::EnvironmentDebug::new(Default::default())})
    }
    pub fn effective_light(&self,base:&Lighting)->Lighting {
        let mut light=self.environment.effective_light(base);light.lod_pixels=self.lod_pixels;
        light.ev100=(light.ev100+self.exposure_offset).clamp(-4.0,24.0);
        if !self.fog {light.fog_density=0.0;light.fog_height_m=base.fog_height_m.max(1.0);light.fog_culling=false;}
        light
    }
    pub fn take_history_reset(&mut self)->bool {std::mem::take(&mut self.history_dirty)}
    pub fn take_sky_request(&mut self)->Option<(&'static str,Value)> {self.environment.take_sky_request()}
    pub fn sync_sky(&mut self,settings:crate::procedural_sky::LiveSettings,status:&str) {
        if self.environment.sync_sky(settings,status) {self.refresh(true);}
    }
    pub fn take_tree_apply(&mut self)->Option<crate::tree_tuning::Settings> {
        std::mem::take(&mut self.tree_apply_requested).then_some(self.tree_pending)
    }
    pub fn applied_tree_settings(&self)->crate::tree_tuning::Settings {self.tree_applied}
    pub fn tree_committed(&mut self,settings:crate::tree_tuning::Settings,report:Value) {
        self.tree_applied=settings;self.tree_pending=settings;self.tree_apply_requested=false;self.tree_report=report;
        self.tree_status=if self.tree_report["editing_available"]==false {"Authored scene retained; tree editing is unavailable for this experimental or tree-free scene.".into()}
            else{format!("Applied: {} / {} trees; shadows and native trunk collisions rebuilt.",
                self.tree_report["selected_source_trees"],self.tree_report["total_source_trees"])};self.refresh(true);
    }
    pub fn tree_failed(&mut self,error:&str) {self.tree_status=format!("Could not apply: {error}");self.refresh(true);}
    pub fn begin_frame(&mut self)->(Instant,Option<f64>) {
        self.cpu_execution=Some(render::cpu_time::ThreadCpuTimer::start());
        let now=Instant::now();let cadence=self.cpu_start.replace(now).map(|last|now.duration_since(last).as_secs_f64()*1000.0);
        (now,cadence)
    }
    pub fn end_frame(&mut self,frame:u64,start:Instant,cadence:Option<f64>,rendered:bool) {
        let execution_ms=self.cpu_execution.take().and_then(|timer|timer.elapsed_ms());
        self.current_frame=frame;
        if rendered && self.ui.open {self.timing.record_cpu(CpuFrameTiming {frame_id:frame,
            frame_body_ms:Some(start.elapsed().as_secs_f64()*1000.0),execution_ms,cadence_ms:cadence});}
    }
    pub fn ingest(&mut self,drain:&GpuProfileDrain,frame:u64) {
        self.current_frame=frame;
        if !self.ui.open {return;}
        let clean=GpuProfileDrain {samples:drain.samples.iter().filter(|s|s.frame_id>self.cutoff_frame).cloned().collect(),
            pending:drain.pending,dropped:drain.dropped,errors:drain.errors,enabled:drain.enabled,supported:drain.supported};
        self.timing.ingest_gpu(&clean,frame);
    }
    /// An old renderer's samples cannot describe a new stage or feature set.
    pub fn reset_measurements(&mut self,frame:u64) {
        self.cutoff_frame=frame;self.current_frame=frame;self.timing=TimingModel::new(32);self.model_at=None;
    }
    fn refresh(&mut self,force:bool) {
        if !force && self.model_at.is_some_and(|at|at.elapsed()<Duration::from_millis(250)){return;}
        let snap=self.timing.snapshot(self.current_frame);
        let mut controls=Vec::new();
        let mut toggle=|id,label:&str,value,help:&str|controls.push(DebugControl {id,label:label.into(),value:DebugValue::Toggle(value),enabled:true,menu:Some(match id {GRASS=>GRASS_MENU,HEADLIGHTS=>105,_=>ADVANCED_MENU}),advanced:false,help:help.into()});
        toggle(GRASS,"Grass",self.options.meadow_enabled,"Skips procedural grass rendering; restores source and distant layers together.");
        toggle(HEADLIGHTS,"Vehicle lights",self.options.headlights_enabled,"Skips headlight shadows and illumination plus tail, brake and reverse illumination. Exhaust flashes have their own switch.");
        toggle(GLASS,"Transparency",self.options.transparency_enabled,"Skips the transparent-material pass, including car windows.");
        toggle(EXHAUST,"Exhaust",self.options.exhaust_enabled,"Skips exhaust smoke, fire and heat-distortion rendering.");
        toggle(PARTICLES,"Wheel particles",self.options.wheel_particles_enabled,"Skips the wheel debris and dust draw pass.");
        let mut number=|id,label:&str,value,min,max,step,decimals,unit:&str,help:&str|controls.push(DebugControl {id,label:label.into(),
            value:DebugValue::Number {value,min,max,step,decimals,unit:unit.into()},enabled:true,
            menu:match id {EXPOSURE=>Some(105),LOD=>Some(ADVANCED_MENU),TREE_SIZE..=TREE_BLUE=>Some(TREES_MENU),_=>Some(GRASS_MENU)},advanced:false,help:help.into()});
        number(DISTANCE,"Grass draw distance",f64::from(self.options.meadow_distance_scale)*100.0,25.0,100.0,5.0,0,"%","Scales both grass representations within allocated range. 100% = 80 m source / 180 m far. Density is unchanged.");
        number(LOD,"Geometry LOD error",f64::from(self.lod_pixels),0.5,4.0,0.25,2,"px","Larger error permits simpler tree, rock and other cluster meshes. Grass uses its separate source LOD.");
        number(EXPOSURE,"Exposure offset",f64::from(self.exposure_offset),-3.0,3.0,0.25,2,"EV","Positive EV darkens the image; negative EV brightens it. Relative to the selected time of day.");
        number(GRASS_SIZE,"Plant size",f64::from(self.options.meadow_size_scale)*100.0,50.0,200.0,5.0,0,"%","Live size around each fixed root; near and distant grass use the same scale.");
        number(GRASS_COUNT,"Patch count",f64::from(self.options.meadow_density)*100.0,0.0,100.0,5.0,0,"%","Live percentage of allocated grass patches. 100% is the full authored population; roots remain stable.");
        for (id,label,i) in [(GRASS_RED,"Tint red",0),(GRASS_GREEN,"Tint green",1),(GRASS_BLUE,"Tint blue",2)] {
            number(id,label,f64::from(self.options.meadow_color_multiplier[i])*100.0,0.0,200.0,5.0,0,"%","Live linear-albedo multiplier shared by close grass, distant grass and substrate. 100% keeps the source color.");
        }
        number(GRASS_LOD,"Grass LOD error",f64::from(self.options.meadow_lod_pixels),0.5,4.0,0.25,2,"px","Higher error selects simpler blade meshes sooner. It does not reduce patch count or change the source-to-distant transition.");
        number(TREE_SIZE,"Tree size",f64::from(self.tree_pending.size_scale)*100.0,50.0,150.0,5.0,0,"%","Staged: scales trees around their roots, including trunk collisions. Press Apply tree changes.");
        number(TREE_COUNT,"Tree count",f64::from(self.tree_pending.density)*100.0,0.0,100.0,5.0,0,"%","Staged: stable subset of the existing forest, near and far. 100% restores all authored trees.");
        for (id,label,i) in [(TREE_RED,"Crown tint red",0),(TREE_GREEN,"Crown tint green",1),(TREE_BLUE,"Crown tint blue",2)] {
            number(id,label,f64::from(self.tree_pending.color_multiplier[i])*100.0,25.0,200.0,5.0,0,"%","Staged leaf/needle albedo multiplier for the shared near/far source. Bark retains its original color.");
        }
        let dirty=self.tree_pending!=self.tree_applied;
        controls.push(DebugControl{id:TREE_APPLY,label:if dirty {"Apply tree changes *"}else{"Apply tree changes"}.into(),value:DebugValue::Button,enabled:dirty,menu:Some(TREES_MENU),advanced:false,help:self.tree_status.clone()});
        controls.push(DebugControl{id:TREE_DISCARD,label:"Discard pending changes".into(),value:DebugValue::Button,enabled:dirty,menu:Some(TREES_MENU),advanced:false,help:"Restore the fields to the currently rendered tree settings, without rebuilding.".into()});
        controls.push(DebugControl {id:FOG,label:"Distance fog".into(),value:DebugValue::Toggle(self.fog),enabled:true,menu:Some(104),advanced:false,help:"Disables distance extinction and its visibility culling. The procedural sky remains.".into()});
        controls.extend(self.environment.controls());
        controls.push(DebugControl {id:BLOOM,label:"Highlight bloom".into(),value:DebugValue::Toggle(self.options.bloom_enabled),enabled:true,
            menu:Some(105),advanced:false,help:"Small screen-space halo from exposed highlights. No extra light, fog or scene brightness.".into()});
        controls.push(DebugControl {id:BLOOM_STRENGTH,label:"Bloom strength".into(),value:DebugValue::Number {value:f64::from(self.options.bloom_strength)*100.0,
            min:0.0,max:20.0,step:1.0,decimals:0,unit:"%".into()},enabled:true,menu:Some(105),advanced:false,
            help:"4% default. Quarter-resolution filtering is measured separately as bloom; 0% skips its GPU passes.".into()});
        let mut menus=vec![
            DebugMenu{id:VEGETATION_MENU,parent:None,label:"Vegetation".into(),help:"Separate grass and tree controls. Grass edits are live; tree edits require Apply.".into()},
            DebugMenu{id:GRASS_MENU,parent:Some(VEGETATION_MENU),label:"Grass - live".into(),help:"Size, patch count and color update live across near and distant grass.".into()},
            DebugMenu{id:TREES_MENU,parent:Some(VEGETATION_MENU),label:"Trees - apply changes".into(),help:"Apply rebuilds trees and collisions. Applied values carry to new stages; Reset restores the saved Sideways baseline.".into()},
            DebugMenu{id:ADVANCED_MENU,parent:None,label:"Advanced rendering".into(),help:"Feature switches and geometry detail diagnostics.".into()},
        ];
        menus.extend(crate::debug_environment::EnvironmentDebug::menus());
        self.model=DebugModel {controls,menus,gpu_interval_ms:snap.gpu_interval_ms,gpu_active_ms:snap.gpu_union_ms,
            cpu_frame_ms:snap.cpu_frame_ms,cpu_execution_ms:snap.cpu_execution_ms,gpu_age_frames:snap.gpu_age_frames,
            gpu_passes:snap.gpu_passes.iter().map(|p|DebugPass {label:p.name.to_owned(),ms:p.mean_ms}).collect(),
            status:Some(if self.gpu_unavailable {"GPU timestamps unavailable; CPU timing remains available.".into()}
                else {format!("32-sample mean | {:?} | {} GPU samples | {} pending / {} dropped",snap.gpu_state,snap.gpu_samples,snap.pending,snap.dropped)}),
            ..Default::default()};
        self.model_at=Some(Instant::now());
    }
    fn apply(&mut self,action:DebugAction) {
        if matches!(action,DebugAction::ResetDefaults) {self.environment.reset_lighting();}
        match self.environment.apply(action) {
            Ok(crate::debug_environment::Change::Unhandled)=>{},
            Ok(change)=>{self.changes+=1;
                if matches!(change,crate::debug_environment::Change::Lighting) {self.history_dirty=true;self.reset_measurements(self.current_frame);}
                self.refresh(true);return;},
            Err(error)=>{eprintln!("debug environment: {error}");return;},
        }
        let staged=matches!(action,DebugAction::SetNumber{id:TREE_SIZE..=TREE_BLUE,..}|DebugAction::Activate{..});
        let post_process=matches!(action,DebugAction::SetToggle{id:BLOOM,..}|DebugAction::SetNumber{id:BLOOM_STRENGTH,..});
        match action {
            DebugAction::SetToggle{id,value}=>match id {GRASS=>self.options.meadow_enabled=value,HEADLIGHTS=>self.options.headlights_enabled=value,
                GLASS=>self.options.transparency_enabled=value,EXHAUST=>self.options.exhaust_enabled=value,
                PARTICLES=>self.options.wheel_particles_enabled=value,FOG=>self.fog=value,BLOOM=>self.options.bloom_enabled=value,_=>return},
            DebugAction::SetNumber{id,value} if value.is_finite()=>match id {
                DISTANCE=>self.options.meadow_distance_scale=(value/100.0).clamp(0.25,1.0) as f32,
                GRASS_SIZE=>self.options.meadow_size_scale=(value/100.0).clamp(0.5,2.0) as f32,
                GRASS_COUNT=>self.options.meadow_density=(value/100.0).clamp(0.0,1.0) as f32,
                GRASS_RED..=GRASS_BLUE=>self.options.meadow_color_multiplier[(id-GRASS_RED) as usize]=(value/100.0).clamp(0.0,2.0) as f32,
                TREE_SIZE=>self.tree_pending.size_scale=(value/100.0).clamp(0.5,1.5) as f32,
                TREE_COUNT=>self.tree_pending.density=(value/100.0).clamp(0.0,1.0) as f32,
                TREE_RED..=TREE_BLUE=>self.tree_pending.color_multiplier[(id-TREE_RED) as usize]=(value/100.0).clamp(0.25,2.0) as f32,
                LOD=>self.lod_pixels=value.clamp(0.5,4.0) as f32,
                GRASS_LOD=>self.options.meadow_lod_pixels=value.clamp(0.5,4.0) as f32,
                BLOOM_STRENGTH=>self.options.bloom_strength=(value/100.0).clamp(0.0,0.2) as f32,
                EXPOSURE=>self.exposure_offset=value.clamp(-3.0,3.0) as f32,_=>return},
            DebugAction::Activate{id:TREE_APPLY}=>self.tree_apply_requested=self.tree_pending!=self.tree_applied,
            DebugAction::Activate{id:TREE_DISCARD}=>self.tree_pending=self.tree_applied,
            DebugAction::ResetDefaults=>{self.options=self.baseline.grass;self.lod_pixels=1.0;self.exposure_offset=0.0;self.fog=true;
                self.tree_pending=self.baseline.trees;self.tree_apply_requested=self.tree_applied!=self.tree_pending;},
            _=>return,
        }
        self.changes+=1;
        if !staged {if !post_process {self.history_dirty=true;}self.reset_measurements(self.current_frame);}
        self.refresh(true);
    }
    pub fn key(&mut self,key:DebugKey,size:[u32;2])->bool {
        self.refresh(false);let was_open=self.ui.open;
        let result=self.ui.on_key(key,size,&self.model);
        if let Some(action)=result.action {self.apply(action);}
        if was_open!=self.ui.open {self.reset_measurements(self.current_frame);}
        result.consumed
    }
    pub fn pointer(&mut self,position:[f32;2]) {self.ui.pointer_move(position);}
    pub fn release(&mut self,size:[u32;2]) {
        // Pointer move + release can both arrive between rendered frames.
        // Commit the final drag position before dropping capture.
        self.tick(0.0,size);self.ui.pointer_up();
    }
    pub fn cancel_interaction(&mut self) {self.ui.cancel_interaction();}
    pub fn tick(&mut self,seconds:f64,size:[u32;2]) {
        self.refresh(false);
        let response=self.ui.tick(seconds,size,&self.model);
        if let Some(action)=response.action {self.apply(action);}
    }
    pub fn click(&mut self,size:[u32;2])->bool {
        self.refresh(false);let result=self.ui.on_pointer_down(size,&self.model);
        if let Some(action)=result.action {self.apply(action);}result.consumed
    }
    pub fn scroll(&mut self,lines:f32,size:[u32;2])->bool {self.refresh(false);self.ui.on_scroll(lines,size,&self.model)}
    pub fn script_frame(&mut self,frame:u64,size:[u32;2]) {
        self.current_frame=frame.saturating_sub(1);
        while self.script.front().is_some_and(|(at,_)|*at<=frame) {
            let (_,event)=self.script.pop_front().unwrap();
            match event {ScriptEvent::Key(key)=>{self.key(key,size);},ScriptEvent::Pointer(p)=>self.pointer(p),
                ScriptEvent::Down=>{self.click(size);},ScriptEvent::Up=>self.release(size)}
        }
    }
    pub fn draw(&mut self,list:&mut HudDrawList) {self.refresh(false);self.ui.draw(list,&self.model);}
    pub fn metadata(&self)->Value {
        let timing=self.timing.snapshot(self.current_frame);
        let mut metadata=json!({"open":self.ui.open,"changes":self.changes,"meadow":self.options.meadow_enabled,
            "menu":self.ui.current_menu(),
            "headlights":self.options.headlights_enabled,"transparency":self.options.transparency_enabled,
            "exhaust":self.options.exhaust_enabled,"wheel_particles":self.options.wheel_particles_enabled,
            "grass_distance_scale":self.options.meadow_distance_scale,"lod_pixels":self.lod_pixels,
            "grass_lod_pixels":self.options.meadow_lod_pixels,
            "grass_size_scale":self.options.meadow_size_scale,"grass_patch_count_fraction":self.options.meadow_density,
            "grass_color_multiplier":self.options.meadow_color_multiplier,
            "tree_pending":{"size_scale":self.tree_pending.size_scale,"density":self.tree_pending.density,"color_multiplier":self.tree_pending.color_multiplier},
            "tree_applied":{"size_scale":self.tree_applied.size_scale,"density":self.tree_applied.density,"color_multiplier":self.tree_applied.color_multiplier},
            "tree_report":self.tree_report,"tree_status":self.tree_status,
            "saved_vegetation_baseline":{"source":self.baseline.origin.label(),"tree_size_scale":self.baseline.trees.size_scale,
                "tree_population_fraction":self.baseline.trees.density,"tree_leaf_rgb":self.baseline.trees.color_multiplier,
                "grass_size_scale":self.baseline.grass.meadow_size_scale,"grass_patch_count_fraction":self.baseline.grass.meadow_density,
                "grass_rgb":self.baseline.grass.meadow_color_multiplier,"grass_lod_pixels":self.baseline.grass.meadow_lod_pixels,
                "grass_distance_scale":self.baseline.grass.meadow_distance_scale},
            "environment":self.environment.metadata(),
            "exposure_offset_ev":self.exposure_offset,"fog":self.fog,"pending_script_events":self.script.len(),
            "gpu_state":format!("{:?}",timing.gpu_state),"gpu_samples":timing.gpu_samples,
            "gpu_interval_ms":timing.gpu_interval_ms,"gpu_union_ms":timing.gpu_union_ms,
            "gpu_age_frames":timing.gpu_age_frames,"cpu_frame_ms":timing.cpu_frame_ms,
            "cpu_execution_ms":timing.cpu_execution_ms,"cpu_execution_samples":timing.cpu_execution_samples,
            "cpu_last_execution_ms":timing.cpu_last_execution_ms,
            "scope":"Sideways saved vegetation baseline with process-local edits. Applied trees carry to new stages; Reset uses the saved baseline. CPU execution is main-thread user+kernel time, excludes waits and workers; frame wall includes waits. GPU intervals exclude HUD/presentation. Grass is live; trees rebuild renderer/collisions."});
        metadata["bloom"]=json!({"enabled":self.options.bloom_enabled,"strength":self.options.bloom_strength,
            "active":render::bloom::active(self.options.bloom_enabled,self.options.bloom_strength),
            "threshold_exposed":render::bloom::THRESHOLD,"knee_exposed":render::bloom::KNEE,
            "scope":"Three quarter-resolution per-view filters after reconstruction/effects, before AgX; GPU interval bloom excludes combine in tonemap."});
        metadata
    }
}

fn load_script(path:&str)->Result<VecDeque<(u64,ScriptEvent)>,String> {
    let metadata=std::fs::metadata(path).map_err(|e|format!("debug UI script: {e}"))?;
    if metadata.len()>65_536 {return Err("debug UI script exceeds 64 KiB".into());}
    let bytes=std::fs::read(path).map_err(|e|format!("debug UI script: {e}"))?;
    let value:Value=serde_json::from_slice(&bytes).map_err(|e|format!("debug UI script: {e}"))?;
    parse_script(&value)
}
fn parse_script(value:&Value)->Result<VecDeque<(u64,ScriptEvent)>,String> {
    let rows=value.as_array().ok_or("debug UI script must be an array of frame/input events")?;
    if rows.len()>256 {return Err("debug UI script maximum is 256 events".into());}
    let mut result=VecDeque::new();let mut previous=0;
    for row in rows {
        let frame=row["frame"].as_u64().ok_or("debug UI event requires integer frame")?;
        if frame==0 || frame>4096 || frame<previous {return Err("debug UI frames must be sorted in 1..4096".into());}
        if ["key","pointer","pointer_down","pointer_up"].iter().filter(|k|row.get(**k).is_some()).count()!=1 {return Err("debug UI event requires exactly one input action".into());}
        let event=if let Some(key)=row.get("key") {ScriptEvent::Key(match key.as_str().ok_or("debug UI key must be a string")? {
            "toggle"=>DebugKey::Toggle,"close"=>DebugKey::Close,"up"=>DebugKey::Up,"down"=>DebugKey::Down,
            "left"=>DebugKey::Left,"right"=>DebugKey::Right,"enter"=>DebugKey::Enter,
            "page_up"=>DebugKey::PageUp,"page_down"=>DebugKey::PageDown,_=>return Err("unknown debug UI script key".into())})}
        else if let Some(pointer)=row.get("pointer") {
            let p: [f32;2]=serde_json::from_value(pointer.clone()).map_err(|_|"debug UI pointer requires two finite coordinates")?;
            if !p.iter().all(|x|x.is_finite()&&x.abs()<=32768.0) {return Err("debug UI pointer outside bounds".into());}
            ScriptEvent::Pointer(p)
        } else if row["pointer_down"]==true {ScriptEvent::Down}
        else if row["pointer_up"]==true {ScriptEvent::Up}
        else {return Err("debug UI pointer press/release must be true".into());};
        result.push_back((frame,event));previous=frame;
    }
    Ok(result)
}

#[cfg(test)]mod tests {
    use super::*;
    fn neutral(open:bool)->EngineDebug {
        EngineDebug::with_baseline(open,None,crate::render_baseline::Preset::authored()).unwrap()
    }
    fn open_grass(ui:&mut EngineDebug,size:[u32;2]) {
        for key in [DebugKey::Enter,DebugKey::Down,DebugKey::Enter] {ui.key(key,size);}
        assert_eq!(ui.ui.current_menu(),Some(GRASS_MENU));
    }
    #[test]fn drag_release_between_frames_applies_final_pointer_position() {
        let size=[1920,1080];let mut ui=neutral(true);let mut list=HudDrawList::new(size);
        open_grass(&mut ui,size);ui.draw(&mut list);
        let value=list.text.iter().find(|t|t.text=="100%").unwrap();
        let start=[value.position[0]+value.width*0.5,value.position[1]+10.0];
        ui.pointer(start);ui.click(size);ui.pointer([start[0]-24.0,start[1]]);ui.release(size);
        assert!((ui.options.meadow_distance_scale-0.9).abs()<1e-6);
        ui.tick(2.0,size);assert!((ui.options.meadow_distance_scale-0.9).abs()<1e-6);
    }
    #[test]fn debug_changes_are_bounded_resettable_and_do_not_mutate_base_lighting() {
        let mut ui=neutral(false);let mut light=Lighting::default();light.ev100=6.0;light.fog_density=0.02;
        ui.apply(DebugAction::SetNumber{id:EXPOSURE,value:-2.0});ui.apply(DebugAction::SetToggle{id:FOG,value:false});
        let changed=ui.effective_light(&light);assert_eq!(changed.ev100,4.0);assert_eq!(changed.fog_density,0.0);assert_eq!(light.ev100,6.0);
        ui.apply(DebugAction::SetNumber{id:DISTANCE,value:1e6});assert_eq!(ui.options.meadow_distance_scale,1.0);
        ui.apply(DebugAction::SetNumber{id:DISTANCE,value:f64::NAN});assert_eq!(ui.options.meadow_distance_scale,1.0);
        ui.apply(DebugAction::ResetDefaults);assert_eq!(ui.effective_light(&light).fog_density,0.02);assert_eq!(ui.effective_light(&light).ev100,6.0);
        assert_eq!(ui.options,DebugRenderOptions::default());
    }
    #[test]fn scripted_keys_use_real_ui_action_path_and_can_restore_grass() {
        let mut ui=neutral(true);
        open_grass(&mut ui,[1920,1080]);ui.key(DebugKey::Down,[1920,1080]);
        ui.script=parse_script(&json!([{"frame":1,"key":"enter"},{"frame":2,"key":"enter"}])).unwrap();
        ui.script_frame(1,[1920,1080]);assert!(!ui.options.meadow_enabled);
        ui.script_frame(2,[1920,1080]);assert!(ui.options.meadow_enabled);assert_eq!(ui.changes,2);
        for bad in [json!([{"frame":0,"key":"enter"}]),json!([{"frame":2,"key":"enter"},{"frame":1,"key":"enter"}]),json!([{"frame":1,"key":"unknown"}])] {
            assert!(parse_script(&bad).is_err());
        }
    }
    #[test]fn bloom_controls_reset_without_mutating_light_or_temporal_history() {
        let mut ui=neutral(true);let light=Lighting::default();let before=ui.effective_light(&light);
        ui.apply(DebugAction::SetToggle{id:BLOOM,value:false});
        assert!(!ui.options.bloom_enabled);assert!(!ui.take_history_reset());
        ui.apply(DebugAction::SetNumber{id:BLOOM_STRENGTH,value:999.0});
        assert_eq!(ui.options.bloom_strength,0.2);assert!(!ui.take_history_reset());
        ui.apply(DebugAction::SetNumber{id:BLOOM_STRENGTH,value:f64::NAN});assert_eq!(ui.options.bloom_strength,0.2);
        assert_eq!(ui.effective_light(&light).ev100,before.ev100);assert_eq!(ui.effective_light(&light).sun_lux,before.sun_lux);
        assert_eq!(ui.metadata()["bloom"]["active"],false);
        ui.apply(DebugAction::ResetDefaults);assert!(ui.options.bloom_enabled);assert_eq!(ui.options.bloom_strength,0.04);
        assert!(ui.model.controls.iter().filter(|c|matches!(c.id,BLOOM|BLOOM_STRENGTH)).all(|c|c.menu==Some(105)));
    }
    #[test]fn vegetation_parameters_are_bounded_and_tree_edits_wait_for_apply() {
        let mut ui=neutral(true);
        ui.apply(DebugAction::SetNumber{id:GRASS_SIZE,value:10000.0});
        ui.apply(DebugAction::SetNumber{id:GRASS_COUNT,value:-100.0});
        ui.apply(DebugAction::SetNumber{id:GRASS_BLUE,value:150.0});
        ui.apply(DebugAction::SetNumber{id:GRASS_LOD,value:2.0});
        assert_eq!(ui.options.meadow_size_scale,2.0);assert_eq!(ui.options.meadow_density,0.0);
        assert_eq!(ui.options.meadow_color_multiplier,[1.0,1.0,1.5]);assert!(ui.options.validate().is_ok());
        assert_eq!(ui.options.meadow_lod_pixels,2.0);assert_eq!(ui.lod_pixels,1.0);
        assert_eq!(ui.metadata()["grass_lod_pixels"],2.0);
        ui.apply(DebugAction::SetNumber{id:TREE_COUNT,value:50.0});
        assert!(ui.take_tree_apply().is_none());assert_eq!(ui.tree_applied.density,1.0);
        ui.apply(DebugAction::Activate{id:TREE_APPLY});let pending=ui.take_tree_apply().unwrap();
        assert_eq!(pending.density,0.5);ui.tree_committed(pending,json!({"test":true}));
        ui.apply(DebugAction::SetNumber{id:TREE_SIZE,value:125.0});ui.apply(DebugAction::Activate{id:TREE_DISCARD});
        assert_eq!(ui.tree_pending,ui.tree_applied);assert_eq!(ui.tree_pending.size_scale,1.0);
        ui.apply(DebugAction::ResetDefaults);assert_eq!(ui.take_tree_apply(),Some(Default::default()));
        assert_eq!(ui.options,DebugRenderOptions::default());
    }

    #[test]fn saved_baseline_initializes_ui_and_reset_while_new_stages_keep_applied_edits() {
        let baseline=crate::render_baseline::Preset::parse(include_str!("../../data/render-baseline.toml")).unwrap();
        let mut ui=EngineDebug::with_baseline(false,None,baseline).unwrap();
        assert_eq!(ui.options,baseline.grass);assert_eq!(ui.applied_tree_settings(),baseline.trees);
        let changed=crate::tree_tuning::Settings{size_scale:1.25,density:0.6,color_multiplier:[0.9,1.2,1.0]};
        ui.tree_committed(changed,json!({"selected_source_trees":6,"total_source_trees":10}));
        // New-stage worker receives applied settings, never uncommitted input.
        ui.apply(DebugAction::SetNumber{id:TREE_COUNT,value:20.0});
        let carried=ui.applied_tree_settings();assert_eq!(carried,changed);
        ui.tree_committed(carried,json!({"selected_source_trees":12,"total_source_trees":20}));
        assert_eq!(ui.tree_pending,changed);assert!(ui.take_tree_apply().is_none());
        ui.apply(DebugAction::SetNumber{id:GRASS_COUNT,value:15.0});
        ui.apply(DebugAction::ResetDefaults);
        assert_eq!(ui.options,baseline.grass);assert_eq!(ui.tree_pending,baseline.trees);
        assert_eq!(ui.applied_tree_settings(),changed,"Reset is committed only after scene/physics rebuild succeeds");
        assert_eq!(ui.take_tree_apply(),Some(baseline.trees));
        ui.tree_committed(baseline.trees,json!({}));assert_eq!(ui.applied_tree_settings(),baseline.trees);
        assert_eq!(ui.metadata()["saved_vegetation_baseline"]["grass_patch_count_fraction"],json!(0.55f32));
        assert_eq!(ui.lod_pixels,1.0);assert_eq!(ui.exposure_offset,0.0);
    }
}
