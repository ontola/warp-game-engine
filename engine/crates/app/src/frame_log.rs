//! Opt-in buffered frame diagnostics (`WARP_FRAME_LOG=path`).
use std::{io::Write,sync::{LazyLock,Mutex}};
use web_time::Instant;
struct Log { out:std::io::BufWriter<std::fs::File>, start:Instant, flushed:Instant }
static LOG:LazyLock<Option<Mutex<Log>>>=LazyLock::new(||{
    let path=std::env::var_os("WARP_FRAME_LOG")?;
    let file=std::fs::File::create(path).ok()?;let now=Instant::now();
    Some(Mutex::new(Log{out:std::io::BufWriter::with_capacity(128*1024,file),start:now,flushed:now}))
});
pub fn enabled()->bool { LOG.is_some() }
fn line(kind:&str,values:serde_json::Value) {
    let Some(log)=LOG.as_ref() else{return};let Ok(mut log)=log.lock() else{return};
    let record=serde_json::json!({"kind":kind,"seconds":log.start.elapsed().as_secs_f64(),"data":values});
    let _=serde_json::to_writer(&mut log.out,&record);let _=log.out.write_all(b"\n");
    if log.flushed.elapsed().as_secs_f32()>=0.5 {let _=log.out.flush();log.flushed=Instant::now();}
}
pub fn frame(id:u64,interval:f64,tick:f64,draw:f64,present:f64,height:u32,focused:bool,detail:serde_json::Value) {
    if enabled(){line("frame",serde_json::json!({"id":id,"interval_ms":interval,"tick_ms":tick,"draw_ms":draw,"submit_present_ms":present,"render_height":height,"focused":focused,"detail":detail}));}
}
pub fn gpu(id:u64,ms:f64,passes:&[(&str,f64)]) {
    if enabled(){line("gpu",serde_json::json!({"id":id,"active_ms":ms,"passes":passes}));}
}
