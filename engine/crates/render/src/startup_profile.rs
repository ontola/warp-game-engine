//! Opt-in CPU wall-time attribution for renderer construction. No device poll
//! or GPU wait is added: uploads measure the host call, not GPU completion.
use web_time::Instant;
use std::fmt::Write;
pub use crate::pipeline_cache::Profile as PipelineCacheProfile;

#[derive(Debug, Clone)]
pub struct StartupSection { pub name: &'static str, pub cpu_ms: f64 }

#[derive(Debug, Clone, Default)]
pub struct TextureCacheProfile {
    pub enabled: bool,
    pub hits: u32,
    pub misses: u32,
    pub corrupt: u32,
    pub read_errors: u32,
    pub write_errors: u32,
    pub read_bytes: u64,
    pub written_bytes: u64,
    pub hash_cpu_ms: f64,
    pub read_cpu_ms: f64,
    pub write_cpu_ms: f64,
}

#[derive(Debug, Clone, Default)]
pub struct TextureLayerProfile {
    pub layer: usize,
    pub source_size: [u32; 2],
    pub base_resize_ms: f64,
    pub alpha_analysis_ms: f64,
    pub mip_resize_ms: f64,
    pub coverage_ms: f64,
    pub chain_copy_ms: f64,
    pub linear_rgb_ms: f64,
    pub total_cpu_ms: f64,
    pub output_bytes: u64,
    pub cache: TextureCacheProfile,
}

#[derive(Debug, Clone)]
pub struct TextureArrayProfile {
    pub name: &'static str,
    pub size: u32,
    pub layers: Vec<TextureLayerProfile>,
    pub allocation_cpu_ms: f64,
    pub build_wall_ms: f64,
    pub upload_cpu_ms: f64,
    pub release_cpu_ms: f64,
    pub workers: usize,
}

#[derive(Debug, Clone, Default)]
pub struct StartupProfile {
    pub total_cpu_ms: f64,
    pub sections: Vec<StartupSection>,
    pub textures: Vec<TextureArrayProfile>,
    pub pipeline_cache: Option<PipelineCacheProfile>,
}

impl StartupProfile {
    /// Single structured log line, without adding a serialization dependency to
    /// the renderer. Names are internal static identifiers, never user text.
    pub fn to_json(&self) -> String {
        let mut text = format!("{{\"kind\":\"renderer_startup\",\"scope\":\"CPU wall time; GPU completion excluded; image decoding happens before Renderer::new\",\"total_cpu_ms\":{},\"sections\":[", self.total_cpu_ms);
        for (i,s) in self.sections.iter().enumerate() {
            if i>0 {text.push(',');}
            write!(text,"{{\"name\":\"{}\",\"cpu_ms\":{}}}",s.name,s.cpu_ms).unwrap();
        }
        text.push_str("],\"texture_arrays\":[");
        for (i,a) in self.textures.iter().enumerate() {
            if i>0 {text.push(',');}
            write!(text,"{{\"name\":\"{}\",\"size\":{},\"workers\":{},\"allocation_cpu_ms\":{},\"build_wall_ms\":{},\"upload_cpu_ms\":{},\"release_cpu_ms\":{},\"layers\":[",
                a.name,a.size,a.workers,a.allocation_cpu_ms,a.build_wall_ms,a.upload_cpu_ms,a.release_cpu_ms).unwrap();
            for (j,l) in a.layers.iter().enumerate() {
                if j>0 {text.push(',');}
                write!(text,"{{\"layer\":{},\"source_size\":[{},{}],\"base_resize_ms\":{},\"alpha_analysis_ms\":{},\"mip_resize_ms\":{},\"coverage_ms\":{},\"chain_copy_ms\":{},\"linear_rgb_ms\":{},\"total_cpu_ms\":{},\"output_bytes\":{},",
                    l.layer,l.source_size[0],l.source_size[1],l.base_resize_ms,l.alpha_analysis_ms,l.mip_resize_ms,l.coverage_ms,l.chain_copy_ms,l.linear_rgb_ms,l.total_cpu_ms,l.output_bytes).unwrap();
                let c=&l.cache;
                write!(text,"\"cache\":{{\"enabled\":{},\"hits\":{},\"misses\":{},\"corrupt\":{},\"read_errors\":{},\"write_errors\":{},\"read_bytes\":{},\"written_bytes\":{},\"hash_cpu_ms\":{},\"read_cpu_ms\":{},\"write_cpu_ms\":{}}}}}",
                    c.enabled,c.hits,c.misses,c.corrupt,c.read_errors,c.write_errors,c.read_bytes,c.written_bytes,c.hash_cpu_ms,c.read_cpu_ms,c.write_cpu_ms).unwrap();
            }
            text.push_str("]}");
        }
        text.push_str("],\"pipeline_cache\":");
        text.push_str(&self.pipeline_cache.as_ref().map_or_else(||"null".into(),PipelineCacheProfile::to_json));
        text.push('}'); text
    }
}

/// Profile every renderer built from now on, whatever `KERN_STARTUP_PROFILE`
/// says (a browser has no environment).
static FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn force(on: bool) { FORCED.store(on, std::sync::atomic::Ordering::Relaxed); }

impl StartupProfile {
    /// The `n` slowest sections and texture arrays, one short line.
    pub fn slowest(&self, n: usize) -> String {
        let mut parts: Vec<(f64, String)> = self.sections.iter().map(|s| (s.cpu_ms, s.name.to_string())).collect();
        parts.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut line = format!("{:.0} ms:", self.total_cpu_ms);
        for (ms, name) in parts.iter().take(n) { write!(line, " {name} {ms:.0}").unwrap(); }
        for a in &self.textures {
            write!(line, "; {} {}px {} layers (build {:.0} ms, upload {:.0} ms)", a.name, a.size, a.layers.len(), a.build_wall_ms, a.upload_cpu_ms).unwrap();
        }
        line
    }
}

pub(crate) struct Trace {
    start: Option<Instant>,
    last: Option<Instant>,
    profile: StartupProfile,
}
impl Trace {
    pub fn new() -> Self {
        let start=(crate::switches::STARTUP_PROFILE.on() || FORCED.load(std::sync::atomic::Ordering::Relaxed)).then(Instant::now);
        Self {start,last:start,profile:StartupProfile::default()}
    }
    pub fn enabled(&self)->bool {self.start.is_some()}
    pub fn mark(&mut self,name:&'static str) {
        if let Some(previous)=self.last {
            let now=Instant::now();
            self.profile.sections.push(StartupSection{name,cpu_ms:now.duration_since(previous).as_secs_f64()*1000.});
            self.last=Some(now);
        }
    }
    pub fn texture(&mut self,array:TextureArrayProfile) {if self.enabled(){self.profile.textures.push(array);}}
    pub fn pipeline_cache(&mut self,profile:PipelineCacheProfile) {if self.enabled(){self.profile.pipeline_cache=Some(profile);}}
    pub fn finish(mut self)->Option<StartupProfile> {
        let start=self.start?;
        self.mark("finalize_renderer");
        self.profile.total_cpu_ms=self.last.unwrap().duration_since(start).as_secs_f64()*1000.;
        if !FORCED.load(std::sync::atomic::Ordering::Relaxed) { eprintln!("{}",self.profile.to_json()); }
        Some(self.profile)
    }
}
pub(crate) fn elapsed(start:Option<Instant>)->f64 {start.map_or(0.,|at|at.elapsed().as_secs_f64()*1000.)}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_profile_disjoint_sections_cover_elapsed_wall_time() {
        let now=Instant::now();
        let mut trace=Trace{start:Some(now),last:Some(now),profile:StartupProfile::default()};
        trace.mark("one");trace.mark("two");
        let p=trace.finish().unwrap();
        assert!((p.sections.iter().map(|s|s.cpu_ms).sum::<f64>()-p.total_cpu_ms).abs()<1e-8);
        assert!(p.to_json().contains("\"kind\":\"renderer_startup\""));
    }
    #[test]
    fn startup_profile_disabled_keeps_no_events() {
        let mut trace=Trace{start:None,last:None,profile:StartupProfile::default()};
        trace.mark("ignored"); assert!(trace.profile.sections.is_empty());assert!(trace.finish().is_none());
    }
}
