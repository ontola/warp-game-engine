//! Opt-in elapsed startup timeline. Sequential sections partition main-thread
//! wall time; asynchronous worker measurements are nested detail, not additive.
use std::time::Instant;

pub struct Timeline { clock:Option<Clock> }
struct Clock { start:Instant,last:Instant,sections:Vec<(&'static str,f64)>,complete:bool }
impl Timeline {
    pub fn new()->Self {Self::with_enabled(std::env::var("KERN_STARTUP_PROFILE").as_deref()==Ok("1"))}
    fn with_enabled(enabled:bool)->Self {
        Self {clock:enabled.then(||{let start=Instant::now();Clock {start,last:start,sections:vec![],complete:false}})}
    }
    pub fn mark(&mut self,name:&'static str) {if let Some(clock)=&mut self.clock {clock.mark_at(name,Instant::now());}}
    pub fn complete(&mut self) {if let Some(clock)=&mut self.clock {clock.complete=true;}}
    pub fn emit(&self,event:&str,context:serde_json::Value) {
        if let Some(clock)=&self.clock {eprintln!("{}",clock.record_at(event,context,Instant::now()));}
    }
}
impl Clock {
    fn mark_at(&mut self,name:&'static str,now:Instant) {
        if self.complete{return;}
        self.sections.push((name,now.duration_since(self.last).as_secs_f64()*1000.0));self.last=now;
    }
    fn record_at(&self,event:&str,context:serde_json::Value,now:Instant)->serde_json::Value {
        serde_json::json!({"kind":"cli_startup","schema":1,"event":event,
            "scope":"Elapsed wall time from play::run entry. Sequential sections are disjoint; worker timings in context overlap worker_pending and must not be added. Presentation means submitted, not display scanout. After first scene, startup sections stay frozen; run_end includes gameplay and profile-writer flush, but process/resource teardown is outside this clock.",
            "elapsed_ms":now.duration_since(self.start).as_secs_f64()*1000.0,
            "partition_end_ms":self.last.duration_since(self.start).as_secs_f64()*1000.0,
            "startup_complete":self.complete,"sections":self.sections.iter().map(|(name,wall_ms)|serde_json::json!({"name":name,"wall_ms":wall_ms})).collect::<Vec<_>>(),"context":context})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test] fn sequential_sections_partition_elapsed_and_freeze_after_first_scene() {
        let mut t=Timeline::with_enabled(true);let start=t.clock.as_ref().unwrap().start;
        let c=t.clock.as_mut().unwrap();c.mark_at("scene",start+Duration::from_millis(4));c.mark_at("worker_pending",start+Duration::from_millis(11));
        t.complete();let c=t.clock.as_mut().unwrap();c.mark_at("gameplay",start+Duration::from_millis(80));
        let row=c.record_at("run_end",serde_json::json!({"worker_ms":6}),start+Duration::from_millis(80));
        assert_eq!(row["partition_end_ms"],11.0);assert_eq!(row["elapsed_ms"],80.0);assert_eq!(row["startup_complete"],true);
        assert_eq!(row["sections"].as_array().unwrap().len(),2);
        let sum:f64=row["sections"].as_array().unwrap().iter().map(|s|s["wall_ms"].as_f64().unwrap()).sum();assert_eq!(sum,11.0);
    }
    #[test] fn disabled_timeline_does_not_keep_clock_or_sections() {
        let mut t=Timeline::with_enabled(false);t.mark("scene");t.complete();assert!(t.clock.is_none());
    }
}
