//! Application-owned wgpu pipeline blobs. Vulkan only; never import raw Vulkan
//! data or third-party caches. The driver keys entries by shader + pipeline state,
//! so shader edits add new entries rather than incorrectly reusing old programs.
//! A loaded blob is NOT evidence of a per-pipeline hit (wgpu exposes no hit count).
use sha2::{Digest, Sha256};
use std::{fs::{self, File, OpenOptions}, io::{self, Read, Write, BufReader, BufWriter},
    path::{Path, PathBuf}, sync::{Mutex, OnceLock, atomic::{AtomicU64, Ordering}}};
use web_time::{Instant, SystemTime, UNIX_EPOCH};

const MAGIC: &[u8; 8] = b"WRPPIP01";
const HEADER_BYTES: u64 = 80;
const MAX_BYTES: u64 = 128 * 1024 * 1024;
static TEMP_SERIAL: AtomicU64 = AtomicU64::new(0);

/// Cumulative for this Gpu/device, including subsequent stage constructors.
/// Read/create precede Renderer::new; persist is inside its measured section.
#[derive(Clone, Debug)]
pub struct Profile {
    pub enabled: bool,
    pub supported: bool,
    pub active: bool,
    pub blob_state: &'static str,
    pub save_state: &'static str,
    pub key: String,
    pub loaded_bytes: u64,
    pub written_bytes: u64,
    pub descriptor_requests: u64,
    pub read_verify_cpu_ms: f64,
    pub create_cpu_ms: f64,
    pub extract_cpu_ms: f64,
    pub write_cpu_ms: f64,
    pub write_errors: u64,
}
impl Profile {
    pub fn to_json(&self) -> String {
        // Strings are internal enums/hex, not environment or driver strings.
        format!(concat!("{{\"scope\":\"cumulative device; read/create before renderer; extract/write at explicit savepoints\",",
            "\"enabled\":{},\"supported\":{},\"active\":{},\"blob_state\":\"{}\",\"save_state\":\"{}\",\"key\":\"{}\",",
            "\"loaded_bytes\":{},\"written_bytes\":{},\"descriptor_requests\":{},\"driver_cache_hits\":null,",
            "\"read_verify_cpu_ms\":{},\"create_cpu_ms\":{},\"extract_cpu_ms\":{},\"write_cpu_ms\":{},\"write_errors\":{}}}"),
            self.enabled,self.supported,self.active,self.blob_state,self.save_state,self.key,
            self.loaded_bytes,self.written_bytes,self.descriptor_requests,self.read_verify_cpu_ms,
            self.create_cpu_ms,self.extract_cpu_ms,self.write_cpu_ms,self.write_errors)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key([u8; 32]);
impl Key {
    fn hex(self) -> String { self.0.iter().map(|b| format!("{b:02x}")).collect() }
    fn filename(self) -> String { self.hex() + ".wgpu-cache" }
}
struct State { profile: Profile, saved_requests: u64, saved_digest: Option<[u8;32]> }
pub(crate) struct Cache {
    handle: Option<wgpu::PipelineCache>,
    directory: Option<PathBuf>,
    key: Option<Key>,
    requests: AtomicU64,
    state: Mutex<State>,
}
impl Cache {
    pub fn new(device: &wgpu::Device, info: &wgpu::AdapterInfo) -> Self {
        let enabled = std::env::var("KERN_PIPELINE_CACHE").as_deref() != Ok("0");
        let key = compatibility_key(info, implementation_fingerprint());
        let supported = key.is_some() && device.features().contains(wgpu::Features::PIPELINE_CACHE);
        let directory = if enabled && supported { cache_directory() } else { None };
        let mut profile = Profile { enabled, supported, active:false, blob_state:if !enabled {"disabled"} else if !supported {"unsupported"} else {"no_directory"},
            save_state:"not_saved", key:key.map_or_else(String::new, Key::hex), loaded_bytes:0, written_bytes:0,
            descriptor_requests:0,read_verify_cpu_ms:0.,create_cpu_ms:0.,extract_cpu_ms:0.,write_cpu_ms:0.,write_errors:0 };
        let mut saved_digest = None;
        let handle = if enabled && supported {
            let start = Instant::now();
            let data = directory.as_ref().and_then(|directory| {
                match read_entry(&directory.join(key.unwrap().filename()), key.unwrap()) {
                    Ok(bytes) => { profile.blob_state="loaded"; profile.loaded_bytes=bytes.len() as u64;
                        saved_digest=Some(Sha256::digest(&bytes).into()); Some(bytes) },
                    Err(ReadFailure::Missing) => {profile.blob_state="missing";None},
                    Err(ReadFailure::Corrupt) => {profile.blob_state="corrupt";None},
                    Err(ReadFailure::Io) => {profile.blob_state="read_error";None},
                }
            });
            profile.read_verify_cpu_ms = start.elapsed().as_secs_f64()*1000.;
            let start = Instant::now();
            let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            // SAFETY: None is always permitted. Some contains only this app's
            // get_data() output, persisted in a private local cache namespace.
            // Length, SHA-256, exact adapter/driver/backend/ABI key are checked
            // before this call. wgpu additionally validates its header and Vulkan
            // driver UUID; fallback=true cold-starts on a rejected old format.
            // Like wgpu's documented disk-cache example, this trusts the local
            // user's cache directory; it is not an untrusted asset import API.
            let cache = unsafe { device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                label:Some("Warp shared Vulkan pipelines"), data:data.as_deref(), fallback:true,
            }) };
            let validation_error = pollster::block_on(validation.pop());
            let oom_error = pollster::block_on(oom.pop());
            let error = validation_error.or(oom_error);
            profile.create_cpu_ms = start.elapsed().as_secs_f64()*1000.;
            if let Some(error) = error {
                eprintln!("pipeline cache unavailable; normal compilation continues: {error}");
                profile.blob_state="device_error";None
            } else {profile.active=true;Some(cache)}
        } else {None};
        eprintln!("pipeline cache: enabled={} supported={} active={} blob={} bytes={} (driver hits not observable)",
            profile.enabled,profile.supported,profile.active,profile.blob_state,profile.loaded_bytes);
        Self {handle,directory,key,requests:AtomicU64::new(0),state:Mutex::new(State {profile,saved_requests:0,saved_digest})}
    }

    pub fn handle(&self) -> Option<&wgpu::PipelineCache> {
        if self.handle.is_some() {self.requests.fetch_add(1,Ordering::Relaxed);}
        self.handle.as_ref()
    }

    /// Constructor/reload only: no queue submit, GPU wait, or per-frame I/O.
    /// A shared mutex serializes stage-worker saves on the same device.
    pub fn persist(&self) -> Profile {
        let mut state=self.state.lock().unwrap_or_else(|e|e.into_inner());
        let requested=self.requests.load(Ordering::Relaxed);
        state.profile.descriptor_requests=requested;
        if let (Some(handle),Some(directory),Some(key))=(&self.handle,&self.directory,self.key) {
            if requested != state.saved_requests {
                let start=Instant::now();let data=handle.get_data();
                state.profile.extract_cpu_ms+=start.elapsed().as_secs_f64()*1000.;
                if let Some(data)=data {
                    if data.is_empty() || data.len() as u64>MAX_BYTES {state.profile.save_state="size_rejected";}
                    else {
                        let start=Instant::now();let digest:[u8;32]=Sha256::digest(&data).into();
                        if state.saved_digest==Some(digest) {state.profile.save_state="unchanged";state.saved_requests=requested;}
                        else {match write_entry(directory,key,&data,digest) {
                            Ok(())=>{state.profile.save_state="written";state.profile.written_bytes+=HEADER_BYTES+data.len() as u64;
                                state.saved_digest=Some(digest);state.saved_requests=requested;},
                            Err(e)=>{state.profile.save_state="write_error";state.profile.write_errors+=1;
                                eprintln!("pipeline cache save failed; rendering continues: {e}");},
                        }}
                        state.profile.write_cpu_ms+=start.elapsed().as_secs_f64()*1000.;
                    }
                } else {state.profile.save_state="data_unavailable";}
            }
        }
        state.profile.clone()
    }
}

fn cache_directory() -> Option<PathBuf> {
    if let Some(path)=std::env::var_os("KERN_PIPELINE_CACHE_DIR").filter(|p|!p.is_empty()) {return Some(path.into());}
    if cfg!(target_os="windows") {
        std::env::var_os("LOCALAPPDATA").map(|p|PathBuf::from(p).join("Warp/Cache/pipelines-v1"))
    } else {
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
            .or_else(||std::env::var_os("HOME").map(|p|PathBuf::from(p).join(".cache")))
            .map(|p|p.join("warp/pipelines-v1"))
    }
}

fn implementation_fingerprint() -> &'static [u8;32] {
    static VALUE:OnceLock<[u8;32]>=OnceLock::new();
    VALUE.get_or_init(|| {
        let mut hash=Sha256::new();hash.update(b"warp-pipeline-cache-v1");
        hash.update(std::env::consts::OS.as_bytes());hash.update(std::env::consts::ARCH.as_bytes());
        // Shader/state changes are already cache-keyed by Vulkan. Lock only the
        // pipeline-generating implementation, not unrelated game/audio changes.
        let lock=include_str!("../../../../Cargo.lock");
        for name in ["wgpu","wgpu-core","wgpu-hal","wgpu-types","wgpu-naga-bridge","naga"] {
            let wanted=format!("name = \"{name}\"");
            for package in lock.split("[[package]]").filter(|p|p.lines().any(|l|l.trim()==wanted)) {
                hash.update(package.as_bytes());
            }
        }
        hash.finalize().into()
    })
}

fn compatibility_key(info:&wgpu::AdapterInfo, implementation:&[u8;32]) -> Option<Key> {
    let base=wgpu::util::pipeline_cache_key(info)?;
    let mut hash=Sha256::new();hash.update(implementation);hash.update(base.as_bytes());
    // Exact driver information, device/subgroup details, and backend. wgpu's
    // own header additionally checks driver pipelineCacheUUID during import.
    hash.update(format!("{info:?}").as_bytes());Some(Key(hash.finalize().into()))
}

#[derive(Debug,PartialEq,Eq)]
enum ReadFailure { Missing, Corrupt, Io }
fn read_entry(path:&Path,key:Key)->Result<Vec<u8>,ReadFailure> {
    let file=File::open(path).map_err(|e|if e.kind()==io::ErrorKind::NotFound {ReadFailure::Missing}else{ReadFailure::Io})?;
    let size=file.metadata().map_err(|_|ReadFailure::Io)?.len();
    if !(HEADER_BYTES+1..=HEADER_BYTES+MAX_BYTES).contains(&size) {return Err(ReadFailure::Corrupt);}
    let mut reader=BufReader::new(file);let mut header=[0u8;HEADER_BYTES as usize];
    reader.read_exact(&mut header).map_err(|_|ReadFailure::Corrupt)?;
    let bytes=u64::from_le_bytes(header[40..48].try_into().unwrap());
    if &header[..8]!=MAGIC || header[8..40]!=key.0 || bytes!=size-HEADER_BYTES {return Err(ReadFailure::Corrupt);}
    let mut data=vec![0;bytes as usize];reader.read_exact(&mut data).map_err(|_|ReadFailure::Corrupt)?;
    // A concurrent replacement cannot affect this open handle. Unexpected
    // in-place edits/trailing bytes are rejected, never given to the driver.
    let mut trailing=[0u8;1];
    if reader.read(&mut trailing).map_err(|_|ReadFailure::Io)?!=0 || Sha256::digest(&data)[..]!=header[48..80] {return Err(ReadFailure::Corrupt);}
    Ok(data)
}

fn write_entry(directory:&Path,key:Key,data:&[u8],checksum:[u8;32])->io::Result<()> {
    if data.is_empty() || data.len() as u64>MAX_BYTES {return Err(io::Error::new(io::ErrorKind::InvalidInput,"pipeline cache size"));}
    fs::create_dir_all(directory)?;
    let serial=TEMP_SERIAL.fetch_add(1,Ordering::Relaxed);
    let time=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let temporary=directory.join(format!(".{}.{}.{}.{}.tmp",key.hex(),std::process::id(),time,serial));
    let file=OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result=(|| {
        let mut writer=BufWriter::new(file);writer.write_all(MAGIC)?;writer.write_all(&key.0)?;
        writer.write_all(&(data.len() as u64).to_le_bytes())?;writer.write_all(&checksum)?;writer.write_all(data)?;
        writer.flush()?;writer.get_ref().sync_all()?;drop(writer);
        // Same-filesystem rename replaces the old file atomically, including
        // Windows (MoveFileExW REPLACE_EXISTING). Never delete the old file first.
        // Concurrent processes may last-writer-win valid subsets, affecting only
        // cache usefulness, not correctness; no partially written blob is exposed.
        fs::rename(&temporary,directory.join(key.filename()))
    })();
    if result.is_err() {let _=fs::remove_file(&temporary);}
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDir(PathBuf);
    impl TestDir {fn new()->Self {let path=std::env::temp_dir().join(format!("warp-pipeline-test-{}-{}-{}",std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),TEMP_SERIAL.fetch_add(1,Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();Self(path)}}
    impl Drop for TestDir {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    fn save(dir:&Path,key:Key,data:&[u8]) {write_entry(dir,key,data,Sha256::digest(data).into()).unwrap();}
    fn adapter()->wgpu::AdapterInfo {wgpu::AdapterInfo {name:"test GPU".into(),vendor:1,device:2,
        device_type:wgpu::DeviceType::DiscreteGpu,device_pci_bus_id:"0000:01:00.0".into(),driver:"test".into(),
        driver_info:"1.0".into(),backend:wgpu::Backend::Vulkan,subgroup_min_size:32,subgroup_max_size:32,
        transient_saves_memory:None,limit_bucket:None}}
    #[test]
    fn pipeline_cache_key_separates_driver_device_backend_and_implementation() {
        let a=adapter();let key=compatibility_key(&a,&[1;32]).unwrap();
        assert_eq!(Some(key),compatibility_key(&a.clone(),&[1;32]));
        assert_ne!(Some(key),compatibility_key(&a,&[2;32]));
        for changed in [wgpu::AdapterInfo {driver_info:"2.0".into(),..a.clone()},
            wgpu::AdapterInfo {device:3,..a.clone()},wgpu::AdapterInfo {vendor:4,..a.clone()}] {
            assert_ne!(Some(key),compatibility_key(&changed,&[1;32]));
        }
        assert_eq!(None,compatibility_key(&wgpu::AdapterInfo {backend:wgpu::Backend::Dx12,..a},&[1;32]));
    }
    #[test]
    fn pipeline_cache_envelope_rejects_mutation_truncation_trailing_and_wrong_key() {
        let dir=TestDir::new();let key=Key([3;32]);let data:Vec<u8>=(0..8193).map(|n|(n*37)as u8).collect();
        save(&dir.0,key,&data);let path=dir.0.join(key.filename());let file=fs::read(&path).unwrap();
        assert_eq!(read_entry(&path,key).unwrap(),data);
        assert_eq!(read_entry(&path,Key([4;32])),Err(ReadFailure::Corrupt));
        for index in [0,8,40,48,80,file.len()-1] {let mut changed=file.clone();changed[index]^=1;
            fs::write(&path,changed).unwrap();assert_eq!(read_entry(&path,key),Err(ReadFailure::Corrupt));}
        for length in [0,79,80,file.len()-1] {fs::write(&path,&file[..length]).unwrap();assert_eq!(read_entry(&path,key),Err(ReadFailure::Corrupt));}
        let mut extra=file.clone();extra.push(0);fs::write(&path,extra).unwrap();assert_eq!(read_entry(&path,key),Err(ReadFailure::Corrupt));
    }
    #[test]
    fn pipeline_cache_bounds_checked_before_allocation() {
        let dir=TestDir::new();let key=Key([7;32]);let path=dir.0.join(key.filename());
        let file=File::create(&path).unwrap();file.set_len(MAX_BYTES+HEADER_BYTES+1).unwrap();
        assert_eq!(read_entry(&path,key),Err(ReadFailure::Corrupt));
        assert_eq!(read_entry(&dir.0.join("absent"),key),Err(ReadFailure::Missing));
    }
    #[test]
    fn pipeline_cache_atomic_publication_replaces_and_survives_competing_writers() {
        let dir=TestDir::new();let key=Key([5;32]);save(&dir.0,key,b"old");save(&dir.0,key,b"new");
        assert_eq!(read_entry(&dir.0.join(key.filename()),key).unwrap(),b"new");
        std::thread::scope(|scope| {for value in 0u8..8 {let directory=&dir.0;scope.spawn(move|| {
            save(directory,key,&vec![value;64*1024]);
        });}});
        let final_data=read_entry(&dir.0.join(key.filename()),key).unwrap();
        assert_eq!(final_data.len(),64*1024);assert!(final_data.iter().all(|v|*v==final_data[0]));assert!(final_data[0]<8);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(),1);
        assert!(write_entry(&dir.0,key,&[],[0;32]).is_err());
        assert_eq!(read_entry(&dir.0.join(key.filename()),key).unwrap(),final_data);
    }
}
