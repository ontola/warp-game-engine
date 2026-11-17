//! Reconstructible processed-texture data, never source assets. A cache hit is
//! accepted only with exact metadata/length and a SHA-256 of every mip byte.
use crate::startup_profile::{TextureCacheProfile, elapsed};
use image::RgbaImage;
use sha2::{Digest, Sha256};
use std::{fs::{self, File, OpenOptions}, io::{self, Read, Write, BufReader, BufWriter},
    path::{Path, PathBuf}, sync::{OnceLock, atomic::{AtomicU64,Ordering}}};
use web_time::{SystemTime, UNIX_EPOCH};
use web_time::Instant;

const MAGIC: &[u8;8]=b"WRPMIP01";
const HEADER_BYTES:u64=80;
const ALGORITHM_VERSION:u32=1;
const MAX_TARGET_SIZE:u32=4096;
static TEMP_SERIAL:AtomicU64=AtomicU64::new(0);

#[derive(Clone,Copy)]
pub(crate) struct Settings {
    pub size:u32,
    pub format:wgpu::TextureFormat,
    pub fill:[u8;4],
    pub preserve_alpha:bool,
    pub linear_cutout_rgb:bool,
}
impl Settings {
    fn format_code(self)->Option<u8> {
        match self.format {wgpu::TextureFormat::Rgba8Unorm=>Some(1),wgpu::TextureFormat::Rgba8UnormSrgb=>Some(2),_=>None}
    }
    fn byte_count(self)->Option<u64> {
        if !self.size.is_power_of_two() || self.size>MAX_TARGET_SIZE || self.format_code().is_none(){return None;}
        Some((0..=self.size.ilog2()).map(|m|u64::from(self.size>>m).pow(2)*4).sum())
    }
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) struct Key([u8;32]);
impl Key {
    fn filename(self)->String {self.0.iter().map(|b|format!("{b:02x}")).collect::<String>()+".mips"}
    /// A key for data derived from this entry's mips (compressed blocks),
    /// told apart by `tag` (the encoding and its settings).
    pub fn derived(self,tag:&[u8])->Key {
        let mut h=Sha256::new();h.update(b"warp-derived-texture");h.update(self.0);h.update(tag);Key(h.finalize().into())
    }
}

const BLOCKS_MAGIC:&[u8;8]=b"WRPBLK01";

pub(crate) struct Cache { directory:Option<PathBuf> }
impl Cache {
    pub fn from_env()->Self {
        let enabled=!crate::switches::TEXTURE_CACHE.off();
        let directory=if !enabled {None} else if let Some(path)=crate::switches::TEXTURE_CACHE_DIR.os().filter(|p|!p.is_empty()) {
            Some(PathBuf::from(path))
        } else if cfg!(target_os="windows") {
            std::env::var_os("LOCALAPPDATA").map(|p|PathBuf::from(p).join("Warp/Cache/texture-mips-v1"))
        } else {
            std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
                .or_else(||std::env::var_os("HOME").map(|p|PathBuf::from(p).join(".cache")))
                .map(|p|p.join("warp/texture-mips-v1"))
        };
        Self{directory}
    }

    pub fn load(&self,source:Option<&RgbaImage>,settings:Settings,profile:bool,stats:&mut TextureCacheProfile)
        ->(Option<Vec<RgbaImage>>,Option<Key>) {
        let Some(directory)=self.directory.as_ref() else{return(None,None);};
        let Some(bytes)=settings.byte_count() else{return(None,None);};
        stats.enabled=true;
        let start=profile.then(Instant::now);
        let key=key(source,settings,algorithm_fingerprint());
        stats.hash_cpu_ms+=elapsed(start);
        let start=profile.then(Instant::now);
        let hash_before=stats.hash_cpu_ms;
        let result=read_entry(&directory.join(key.filename()),key,settings,bytes,profile,stats);
        // Verification hashing is separately attributed, not double-counted as I/O.
        stats.read_cpu_ms+=(elapsed(start)-(stats.hash_cpu_ms-hash_before)).max(0.);
        match result {
            Ok(chain)=>{stats.hits+=1;(Some(chain),Some(key))},
            Err(ReadFailure::Missing)=>{stats.misses+=1;(None,Some(key))},
            Err(ReadFailure::Corrupt)=>{stats.misses+=1;stats.corrupt+=1;(None,Some(key))},
            Err(ReadFailure::Io)=>{stats.misses+=1;stats.read_errors+=1;(None,Some(key))},
        }
    }

    pub fn store(&self,key:Key,settings:Settings,chain:&[RgbaImage],profile:bool,stats:&mut TextureCacheProfile) {
        let Some(directory)=self.directory.as_ref() else{return;};
        let start=profile.then(Instant::now);
        let hash_before=stats.hash_cpu_ms;
        let result=write_entry(directory,key,settings,chain,profile,stats);
        stats.write_cpu_ms+=(elapsed(start)-(stats.hash_cpu_ms-hash_before)).max(0.);
        if result.is_err(){stats.write_errors+=1;}
    }
}

impl Cache {
    /// Compressed blocks stored under a derived key: exactly `bytes` long,
    /// checked like a mip entry. Any failure is a miss.
    pub fn load_blocks(&self,key:Key,bytes:usize)->Option<Vec<u8>> {
        let directory=self.directory.as_ref()?;
        let mut file=File::open(directory.join(key.filename())).ok()?;
        if file.metadata().ok()?.len()!=HEADER_BYTES+bytes as u64 {return None;}
        let mut header=[0u8;HEADER_BYTES as usize];file.read_exact(&mut header).ok()?;
        if &header[..8]!=BLOCKS_MAGIC || header[8..40]!=key.0 || u64::from_le_bytes(header[40..48].try_into().unwrap())!=bytes as u64 {return None;}
        let mut data=vec![0u8;bytes];file.read_exact(&mut data).ok()?;
        let actual:[u8;32]=Sha256::digest(&data).into();
        (header[48..80]==actual).then_some(data)
    }

    pub fn store_blocks(&self,key:Key,data:&[u8]) {
        let Some(directory)=self.directory.as_ref() else{return;};
        let _=(||->io::Result<()> {
            fs::create_dir_all(directory)?;
            let serial=TEMP_SERIAL.fetch_add(1,Ordering::Relaxed);
            let time=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
            let temporary=directory.join(format!(".{}.{}.{}.{}.tmp",key.filename(),std::process::id(),time,serial));
            let result=(||{
                let mut writer=BufWriter::new(OpenOptions::new().write(true).create_new(true).open(&temporary)?);
                writer.write_all(BLOCKS_MAGIC)?;writer.write_all(&key.0)?;writer.write_all(&(data.len()as u64).to_le_bytes())?;
                writer.write_all(&<[u8;32]>::from(Sha256::digest(data)))?;writer.write_all(data)?;writer.flush()?;drop(writer);
                fs::rename(&temporary,directory.join(key.filename()))
            })();
            if result.is_err(){let _=fs::remove_file(&temporary);}
            result
        })();
    }
}

fn algorithm_fingerprint()->&'static [u8;32] {
    static VALUE:OnceLock<[u8;32]>=OnceLock::new();
    VALUE.get_or_init(|| {
        let mut h=Sha256::new();h.update(ALGORITHM_VERSION.to_le_bytes());
        h.update(std::env::consts::OS.as_bytes());h.update(std::env::consts::ARCH.as_bytes());
        h.update(include_bytes!("texture_mips.rs"));h.update(include_bytes!("cutout_mips.rs"));
        // Include the locked image implementation version/checksum, rather than
        // invalidating every texture when an unrelated audio dependency changes.
        let lock=include_str!("../../../../Cargo.lock");
        let image=lock.split("[[package]]").find(|p|p.lines().any(|l|l.trim()=="name = \"image\""))
            .expect("locked image package for texture processing");
        h.update(image.as_bytes());h.finalize().into()
    })
}

fn key(source:Option<&RgbaImage>,settings:Settings,algorithm:&[u8;32])->Key {
    let mut h=Sha256::new();h.update(b"warp-processed-texture-mips");h.update(algorithm);
    h.update(settings.size.to_le_bytes());h.update([settings.format_code().unwrap_or(0)]);
    h.update(settings.fill);h.update([settings.preserve_alpha as u8,settings.linear_cutout_rgb as u8]);
    if let Some(source)=source {
        h.update([1]);h.update(source.width().to_le_bytes());h.update(source.height().to_le_bytes());h.update(source.as_raw());
    } else {h.update([0]);}
    Key(h.finalize().into())
}

enum ReadFailure {Missing,Corrupt,Io}
fn read_entry(path:&Path,key:Key,settings:Settings,bytes:u64,profile:bool,stats:&mut TextureCacheProfile)
    ->Result<Vec<RgbaImage>,ReadFailure> {
    let file=File::open(path).map_err(|e|if e.kind()==io::ErrorKind::NotFound{ReadFailure::Missing}else{ReadFailure::Io})?;
    if file.metadata().map_err(|_|ReadFailure::Io)?.len()!=HEADER_BYTES+bytes{return Err(ReadFailure::Corrupt);}
    let mut reader=BufReader::with_capacity(1024*1024,file);
    let mut header=[0u8;HEADER_BYTES as usize];
    reader.read_exact(&mut header).map_err(|_|ReadFailure::Corrupt)?;stats.read_bytes+=HEADER_BYTES;
    if &header[..8]!=MAGIC || header[8..40]!=key.0 || u64::from_le_bytes(header[40..48].try_into().unwrap())!=bytes {
        return Err(ReadFailure::Corrupt);
    }
    let mut checksum=Sha256::new();let mut chain=Vec::with_capacity(settings.size.ilog2()as usize+1);
    for level in 0..=settings.size.ilog2() {
        let size=settings.size>>level;
        let mut data=vec![0u8;size as usize*size as usize*4];
        reader.read_exact(&mut data).map_err(|_|ReadFailure::Corrupt)?;stats.read_bytes+=data.len()as u64;
        let start=profile.then(Instant::now);checksum.update(&data);stats.hash_cpu_ms+=elapsed(start);
        chain.push(RgbaImage::from_raw(size,size,data).ok_or(ReadFailure::Corrupt)?);
    }
    let start=profile.then(Instant::now);let actual:[u8;32]=checksum.finalize().into();stats.hash_cpu_ms+=elapsed(start);
    if header[48..80]!=actual{return Err(ReadFailure::Corrupt);}
    Ok(chain)
}

fn write_entry(directory:&Path,key:Key,settings:Settings,chain:&[RgbaImage],profile:bool,stats:&mut TextureCacheProfile)->io::Result<()> {
    let bytes=settings.byte_count().ok_or_else(||io::Error::other("unsupported processed texture"))?;
    if chain.len()!=settings.size.ilog2()as usize+1 || chain.iter().enumerate().any(|(i,m)|m.dimensions()!=(settings.size>>i,settings.size>>i)) {
        return Err(io::Error::other("invalid mip chain"));
    }
    let start=profile.then(Instant::now);let mut checksum=Sha256::new();
    for mip in chain {checksum.update(mip.as_raw());}
    let checksum:[u8;32]=checksum.finalize().into();stats.hash_cpu_ms+=elapsed(start);
    fs::create_dir_all(directory)?;
    let serial=TEMP_SERIAL.fetch_add(1,Ordering::Relaxed);
    let time=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let temporary=directory.join(format!(".{}.{}.{}.{}.tmp",key.filename(),std::process::id(),time,serial));
    // On a name collision do not delete another writer's temporary file.
    let file=OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result=(|| {
        let mut writer=BufWriter::with_capacity(1024*1024,file);
        writer.write_all(MAGIC)?;writer.write_all(&key.0)?;writer.write_all(&bytes.to_le_bytes())?;writer.write_all(&checksum)?;
        for mip in chain {writer.write_all(mip.as_raw())?;}
        writer.flush()?;drop(writer);
        // Atomic publication on the same filesystem: readers never observe the
        // temporary file. Concurrent writers have the same content-addressed key.
        fs::rename(&temporary,directory.join(key.filename()))?;
        stats.written_bytes+=HEADER_BYTES+bytes;
        Ok(())
    })();
    if result.is_err(){let _=fs::remove_file(&temporary);}
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDir(PathBuf);
    impl TestDir {fn new()->Self {
        let n=TEMP_SERIAL.fetch_add(1,Ordering::Relaxed);
        let p=std::env::temp_dir().join(format!("warp-texture-cache-test-{}-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),n));
        fs::create_dir(&p).unwrap();Self(p)
    }}
    impl Drop for TestDir {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    fn settings()->Settings {Settings{size:16,format:wgpu::TextureFormat::Rgba8UnormSrgb,fill:[255;4],preserve_alpha:true,linear_cutout_rgb:true}}
    fn source()->RgbaImage {RgbaImage::from_fn(9,7,|x,y|image::Rgba([(x*27)as u8,(y*37)as u8,93,if x%3==0{0}else{(y*41)as u8}]))}
    #[test]
    fn texture_cache_real_processing_hit_is_byte_exact_for_all_modes() {
        let dir=TestDir::new();let cache=Cache{directory:Some(dir.0.clone())};let src=source();
        for source in [Some(&src),None] {for format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb] {
            for preserve_alpha in [false,true] {for linear_cutout_rgb in [false,true] {
                let settings=Settings{format,preserve_alpha,linear_cutout_rgb,..settings()};
                let mut stats=TextureCacheProfile::default();let (hit,key)=cache.load(source,settings,true,&mut stats);assert!(hit.is_none());
                let (original,_)=crate::frame::texture_mips::build(source,settings,false,0);
                cache.store(key.unwrap(),settings,&original,true,&mut stats);
                let (hit,_)=cache.load(source,settings,true,&mut stats);assert_eq!(hit.unwrap(),original);
                assert_eq!((stats.hits,stats.misses,stats.corrupt,stats.write_errors),(1,1,0,0));
            }}
        }}
    }
    #[test]
    fn texture_cache_keys_include_content_dimensions_and_every_processing_input() {
        let src=source();let s=settings();let original=key(Some(&src),s,algorithm_fingerprint());
        let mut changed=src.clone();changed.as_mut()[0]^=1;
        assert_ne!(original,key(Some(&changed),s,algorithm_fingerprint()));
        let reshaped=RgbaImage::from_raw(7,9,src.as_raw().clone()).unwrap();assert_ne!(original,key(Some(&reshaped),s,algorithm_fingerprint()));
        for other in [Settings{size:8,..s},Settings{format:wgpu::TextureFormat::Rgba8Unorm,..s},Settings{fill:[0;4],..s},Settings{preserve_alpha:false,..s},Settings{linear_cutout_rgb:false,..s}] {
            assert_ne!(original,key(Some(&src),other,algorithm_fingerprint()));
        }
        assert_ne!(original,key(Some(&src),s,&[19;32]));assert_ne!(original,key(None,s,algorithm_fingerprint()));
    }
    #[test]
    fn texture_cache_corrupt_truncated_wrong_key_and_partial_files_are_misses() {
        let dir=TestDir::new();let cache=Cache{directory:Some(dir.0.clone())};let src=source();let settings=settings();
        let mut stats=TextureCacheProfile::default();let (_,key)=cache.load(Some(&src),settings,false,&mut stats);let key=key.unwrap();
        let (chain,_)=crate::frame::texture_mips::build(Some(&src),settings,false,0);
        cache.store(key,settings,&chain,false,&mut stats);let path=dir.0.join(key.filename());let original=fs::read(&path).unwrap();
        for corrupt in [original[..original.len()-1].to_vec(),{let mut b=original.clone();b[90]^=4;b},{let mut b=original.clone();b[9]^=1;b}] {
            fs::write(&path,corrupt).unwrap();let mut p=TextureCacheProfile::default();assert!(cache.load(Some(&src),settings,false,&mut p).0.is_none());assert_eq!(p.corrupt,1);
            cache.store(key,settings,&chain,false,&mut p);assert_eq!(cache.load(Some(&src),settings,false,&mut p).0.unwrap(),chain);
        }
        fs::remove_file(path).unwrap();fs::write(dir.0.join("unfinished.tmp"),original).unwrap();
        assert!(cache.load(Some(&src),settings,false,&mut stats).0.is_none());
    }
    #[test]
    fn texture_cache_concurrent_publication_never_returns_partial_data() {
        let dir=TestDir::new();let cache=Cache{directory:Some(dir.0.clone())};let src=source();let settings=settings();
        let key=key(Some(&src),settings,algorithm_fingerprint());let (chain,_)=crate::frame::texture_mips::build(Some(&src),settings,false,0);
        std::thread::scope(|scope| {for _ in 0..8 {scope.spawn(|| {let mut stats=TextureCacheProfile::default();cache.store(key,settings,&chain,false,&mut stats);assert_eq!(stats.write_errors,0);});}});
        let mut stats=TextureCacheProfile::default();assert_eq!(cache.load(Some(&src),settings,false,&mut stats).0.unwrap(),chain);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(),1);
    }
    #[test]
    fn texture_cache_blocks_round_trip_and_reject_damage() {
        let dir=TestDir::new();let cache=Cache{directory:Some(dir.0.clone())};
        let key=key(Some(&source()),settings(),algorithm_fingerprint()).derived(b"bc7");
        assert!(cache.load_blocks(key,64).is_none());
        let data:Vec<u8>=(0..64).collect();cache.store_blocks(key,&data);
        assert_eq!(cache.load_blocks(key,64).unwrap(),data);assert!(cache.load_blocks(key,48).is_none());
        let path=dir.0.join(key.filename());let mut bytes=fs::read(&path).unwrap();bytes[100]^=1;fs::write(&path,bytes).unwrap();
        assert!(cache.load_blocks(key,64).is_none());
        assert_ne!(key,key.derived(b"other"));
    }
    #[test]
    fn texture_cache_disabled_or_unwritable_does_not_require_cached_data() {
        let src=source();let settings=settings();let mut stats=TextureCacheProfile::default();
        assert!(Cache{directory:None}.load(Some(&src),settings,true,&mut stats).1.is_none());assert!(!stats.enabled);
        let dir=TestDir::new();let file=dir.0.join("not-a-directory");fs::write(&file,b"keep").unwrap();let cache=Cache{directory:Some(file.clone())};
        let (_,key)=cache.load(Some(&src),settings,false,&mut stats);let (chain,_)=crate::frame::texture_mips::build(Some(&src),settings,false,0);
        cache.store(key.unwrap(),settings,&chain,false,&mut stats);assert_eq!(stats.write_errors,1);assert_eq!(fs::read(file).unwrap(),b"keep");
    }
}
