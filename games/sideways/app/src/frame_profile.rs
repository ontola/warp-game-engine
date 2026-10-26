//! Opt-in frame accounting. CPU spans partition one actual frame-body call;
//! delayed GPU samples are joined by rendered frame ID, never added to CPU.
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs::OpenOptions, io::{BufWriter, Write}, sync::mpsc::{self, SyncSender, TrySendError}, thread::JoinHandle, time::{Duration, Instant}};

const QUEUE_RECORDS:usize=512;

pub struct Profiler {
    sender:Option<SyncSender<Value>>,writer:Option<JoinHandle<Result<(),String>>>,
    epoch:Instant,last_start:Option<Instant>,calls:u64,enqueued:u64,dropped:u64,
    pub gpu_unavailable:bool,
}
impl Profiler {
    pub fn new(path:&str,mut header:Value)->Result<Self,String> {
        let file=OpenOptions::new().write(true).create_new(true).open(path).map_err(|e|format!("frame profile requires a new writable file: {e}"))?;
        header["kind"]="frame_profile_header".into();header["schema"]=1.into();header["target_ms"]=(1000.0/120.0).into();
        header["clock"]="process_monotonic_ms".into();header["writer_queue_records"]=QUEUE_RECORDS.into();
        header["scope"]="CPU sections partition frame-body wall time, not CPU execution cycles; cadence is unclamped entry-to-entry wall time. GPU pass sums exclude HUD/presentation and cannot be added to CPU. JSON packing/enqueue occurs after CPU end; its overhead remains in next cadence. Loading, paused and diagnostic frames are separate.".into();
        let (sender,receiver)=mpsc::sync_channel::<Value>(QUEUE_RECORDS);
        let writer=std::thread::Builder::new().name("frame-profile-writer".into()).spawn(move|| {
            let mut file=BufWriter::with_capacity(256*1024,file);
            writeln!(file,"{header}").map_err(|e|format!("frame profile header: {e}"))?;
            for row in receiver {writeln!(file,"{row}").map_err(|e|format!("frame profile write: {e}"))?;}
            file.flush().map_err(|e|format!("frame profile flush: {e}"))
        }).map_err(|e|format!("frame profile writer: {e}"))?;
        Ok(Self {sender:Some(sender),writer:Some(writer),epoch:Instant::now(),last_start:None,calls:0,enqueued:0,dropped:0,gpu_unavailable:false})
    }
    pub fn begin(&mut self)->Trace {
        let start=Instant::now();self.calls+=1;
        let cadence=self.last_start.replace(start).map(|last|start.duration_since(last));
        Trace {cpu_timer:render::cpu_time::ThreadCpuTimer::start(),call_id:self.calls,start,last:start,epoch:self.epoch,cadence,section:"unattributed",sections:BTreeMap::new(),phase:"unrendered",frame_id:None,diagnostic_sync:false,render_state_hash:None}
    }
    pub fn record(&mut self,row:Value) {
        let Some(sender)=&self.sender else{return;};
        match sender.try_send(row) {Ok(())=>self.enqueued+=1,Err(TrySendError::Full(_)|TrySendError::Disconnected(_))=>self.dropped+=1}
    }
    /// Called only on exit, never from a measured frame. Closing the channel
    /// drains the bounded writer queue; GPU completions are never waited for.
    pub fn finish(&mut self)->Result<(),String> {
        if let Some(sender)=self.sender.take() {
            sender.send(json!({"kind":"frame_profile_footer","calls":self.calls,"records_enqueued":self.enqueued,
                "writer_dropped_records":self.dropped,"gpu_unavailable":self.gpu_unavailable,
                "scope":"Pending GPU samples at exit are not waited for; last gpu_status records their count."})).map_err(|e|format!("frame profile footer: {e}"))?;
        }
        if let Some(writer)=self.writer.take(){writer.join().map_err(|_|"frame profile writer panicked".to_string())??;}
        Ok(())
    }
}
impl Drop for Profiler {fn drop(&mut self){let _=self.finish();}}

pub struct Trace {
    cpu_timer:render::cpu_time::ThreadCpuTimer,
    pub call_id:u64,pub phase:&'static str,pub frame_id:Option<u64>,pub diagnostic_sync:bool,
    pub render_state_hash:Option<String>,
    start:Instant,last:Instant,epoch:Instant,cadence:Option<Duration>,section:&'static str,sections:BTreeMap<&'static str,Duration>,
}
impl Trace {
    pub fn section(&mut self,name:&'static str){self.section_at(name,Instant::now());}
    fn section_at(&mut self,name:&'static str,now:Instant) {
        *self.sections.entry(self.section).or_default()+=now.duration_since(self.last);self.last=now;self.section=name;
    }
    pub fn finish(mut self,mut metadata:Value)->Value {
        let cpu_execution_ms=self.cpu_timer.elapsed_ms();
        let end=Instant::now();self.section_at("unattributed",end);
        metadata["kind"]="cpu_frame".into();metadata["call_id"]=self.call_id.into();metadata["frame_id"]=json!(self.frame_id);
        metadata["phase"]=self.phase.into();metadata["diagnostic_sync"]=self.diagnostic_sync.into();
        metadata["render_state_hash"]=json!(self.render_state_hash);
        metadata["start_ms"]=ms(self.start.duration_since(self.epoch)).into();metadata["end_ms"]=ms(end.duration_since(self.epoch)).into();
        metadata["cpu_frame_ms"]=ms(end.duration_since(self.start)).into();metadata["cadence_ms"]=json!(self.cadence.map(ms));
        metadata["cpu_execution_ms"]=json!(cpu_execution_ms);
        metadata["cpu_execution_scope"]="Calling main-thread user+kernel OS execution; excludes waits, descheduling, worker threads and final JSON packing. Coarse OS counters: use multiple-frame means; null when unavailable.".into();
        metadata["sections_ms"]=json!(self.sections.into_iter().map(|(name,t)|(name,ms(t))).collect::<BTreeMap<_,_>>());metadata
    }
}
fn ms(duration:Duration)->f64 {duration.as_secs_f64()*1000.0}
pub fn section(trace:&mut Option<Trace>,name:&'static str){if let Some(trace)=trace {trace.section(name);}}
pub fn phase(trace:&mut Option<Trace>,name:&'static str){if let Some(trace)=trace {trace.phase=name;}}

/// Same-executable replay checksum; no GPU readback and no diagnostic mask.
pub fn render_state_hash(views:&[render::ViewRect],light:&render::Lighting,poses:&[crate::visuals::CarPose])->String {
    let mut hash=0xcbf29ce484222325u64;
    let mut bytes=|data:&[u8]| {for byte in data {hash^=u64::from(*byte);hash=hash.wrapping_mul(0x100000001b3);}};
    bytes(&(views.len() as u32).to_le_bytes());
    for v in views {
        for n in [v.x,v.y,v.w,v.h] {bytes(&n.to_le_bytes());}
        for n in v.cam.pos.to_array().into_iter().chain(v.cam.target.to_array()).chain([v.cam.fov_y,v.cam.near]) {bytes(&n.to_bits().to_le_bytes());}
    }
    bytes(format!("{light:?}").as_bytes());
    bytes(&(poses.len() as u32).to_le_bytes());
    for p in poses {
        for n in p.pos.to_array().into_iter().chain(p.rot.to_array()).chain(p.color.to_array()).chain([p.ride]) {bytes(&n.to_bits().to_le_bytes());}
        bytes(&(p.model.len() as u32).to_le_bytes());bytes(p.model.as_bytes());
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn replay_checksum_catches_camera_and_lighting_changes() {
        let mut views=vec![render::ViewRect{x:0,y:0,w:1920,h:1080,cam:render::Camera{pos:glam::Vec3::new(1.0,2.0,3.0),target:glam::Vec3::ZERO,fov_y:1.0,near:0.1}}];
        let mut light=render::Lighting::default();
        let baseline=render_state_hash(&views,&light,&[]);
        assert_eq!(baseline,render_state_hash(&views,&light,&[]));
        views[0].cam.pos.x+=0.001;
        assert_ne!(baseline,render_state_hash(&views,&light,&[]));
        views[0].cam.pos.x=1.0;light.sun_dir.x+=0.01;
        assert_ne!(baseline,render_state_hash(&views,&light,&[]));
    }
    #[test] fn cpu_intervals_partition_wall_time_without_overlap() {
        let start=Instant::now();let mut trace=Trace {cpu_timer:render::cpu_time::ThreadCpuTimer::start(),call_id:3,start,last:start,epoch:start,cadence:None,section:"unattributed",sections:BTreeMap::new(),phase:"gameplay",frame_id:Some(7),diagnostic_sync:false,render_state_hash:None};
        trace.section_at("physics",start+Duration::from_millis(1));
        trace.section_at("scene_uploads",start+Duration::from_millis(4));
        trace.section_at("physics",start+Duration::from_millis(6));
        trace.section_at("present_acquire",start+Duration::from_millis(8));
        trace.section_at("unattributed",start+Duration::from_millis(10));
        assert_eq!(trace.sections["physics"],Duration::from_millis(5));
        assert_eq!(trace.sections.values().copied().sum::<Duration>(),Duration::from_millis(10));
        assert_eq!(trace.frame_id,Some(7));
    }
    #[test] fn output_queue_never_blocks_the_frame_and_counts_dropped_records() {
        let (sender,_receiver)=mpsc::sync_channel(1);
        let mut profiler=Profiler {sender:Some(sender),writer:None,epoch:Instant::now(),last_start:None,calls:0,enqueued:0,dropped:0,gpu_unavailable:false};
        profiler.record(json!({"kind":"one"}));profiler.record(json!({"kind":"two"}));
        assert_eq!(profiler.enqueued,1);assert_eq!(profiler.dropped,1);
        profiler.sender=None; // This deliberate full test queue has no writer.
    }
    #[test] fn real_frame_ids_and_first_cadence_are_not_invented_for_loading() {
        let (sender,_receiver)=mpsc::sync_channel(1);
        let mut profiler=Profiler {sender:Some(sender),writer:None,epoch:Instant::now(),last_start:None,calls:0,enqueued:0,dropped:0,gpu_unavailable:false};
        let mut trace=profiler.begin();trace.phase="startup_loading";
        let row=trace.finish(json!({}));assert!(row["frame_id"].is_null());assert!(row["cadence_ms"].is_null());
        assert_eq!(row["phase"],"startup_loading");assert!(profiler.begin().cadence.is_some());profiler.sender=None;
    }
    #[test] fn writer_preserves_json_order_flushes_on_exit_and_refuses_existing_files() {
        let suffix=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("sideways-frame-profile-{}-{suffix}.jsonl",std::process::id()));
        let mut profiler=Profiler::new(path.to_str().unwrap(),json!({"test":true})).unwrap();
        let mut trace=profiler.begin();trace.frame_id=Some(42);trace.phase="gameplay";
        profiler.record(trace.finish(json!({"presented":true})));profiler.finish().unwrap();
        let bytes=std::fs::read(&path).unwrap();
        let rows:Vec<Value>=std::str::from_utf8(&bytes).unwrap().lines().map(|line|serde_json::from_str(line).unwrap()).collect();
        assert_eq!(rows.len(),3);assert_eq!(rows[0]["kind"],"frame_profile_header");
        assert_eq!(rows[1]["frame_id"],42);assert_eq!(rows[2]["writer_dropped_records"],0);
        let cpu=rows[1]["cpu_frame_ms"].as_f64().unwrap();
        let sum:f64=rows[1]["sections_ms"].as_object().unwrap().values().map(|v|v.as_f64().unwrap()).sum();
        assert!((cpu-sum).abs()<1e-8);
        assert!(Profiler::new(path.to_str().unwrap(),json!({})).is_err());assert_eq!(std::fs::read(&path).unwrap(),bytes);
        std::fs::remove_file(path).unwrap();
    }
}
