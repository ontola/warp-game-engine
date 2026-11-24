//! Speaker output: open the default device and pull an [`crate::Source`]
//! in its callback. Without a device the game stays silent.
//!
//! The stream lives on its own thread, which opens it again on the default
//! device when it dies (the display sleeping takes HDMI audio with it, a
//! headset unplugged): the game keeps its [`Speaker`] and its source.
//!
//! In a browser (wasm32) cpal plays through WebAudio on the page's own
//! thread, and the speaker holds the stream itself. Browsers start an
//! AudioContext only after the player clicks or presses a key on the page
//! (the warp page resumes it then, engine/docs/browser.md).
use crate::sfx::{sfx, SfxSender};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

type Shared = Arc<Mutex<Box<dyn crate::Source>>>;

pub struct Speaker {
    stop: Arc<AtomicBool>, thread: Option<std::thread::Thread>, pub rate: f32, pub channels: usize,
    /// System output selected when this speaker was opened.
    pub output_name: String,
    /// Browser: the stream itself (no audio thread there).
    #[cfg(target_arch = "wasm32")]
    _stream: Option<cpal::Stream>,
}

impl Drop for Speaker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = &self.thread { t.unpark(); }
    }
}

// WASAPI's virtual default follows the OS route. Some Windows sessions
// reject its asynchronous activation but allow opening the very same
// IMMDevice directly. Match endpoint IDs, including on reconnect: a failed
// headphone/remote stream must not spill into another physical speaker.
fn with_default_output<T>(mut open: impl FnMut(cpal::Device) -> Result<T, String>) -> Result<T, String> {
    let host = cpal::default_host();
    let default = host.default_output_device().ok_or("no audio output device")?;
    #[cfg(windows)]
    {
        let id = default.id().ok();
        match open(default) {
            Ok(result) => Ok(result),
            Err(first) => {
                let Some(concrete) = id.and_then(|id| host.device_by_id(&id)) else { return Err(first); };
                open(concrete).map_err(|second| format!("{first}; Windows endpoint: {second}"))
            }
        }
    }
    #[cfg(not(windows))]
    open(default)
}

/// Open the default output. Build the source once, even when Windows must
/// retry the very same endpoint's native activation after stream creation fails.
pub fn open(make: impl FnOnce(f32, usize) -> Box<dyn crate::Source>) -> Result<Speaker, String> {
    let mut make = Some(make);
    let mut prepared = None;
    with_default_output(|device| {
        let config = device.default_output_config().map_err(|e| format!("audio config: {e}"))?;
        let rate = config.sample_rate();
        let channels = config.channels() as usize;
        let (source_rate, source_channels, source) = prepared.get_or_insert_with(|| {
            let create = make.take().expect("source is constructed once");
            (rate, channels, Arc::new(Mutex::new(create(rate as f32, channels))))
        });
        if (*source_rate, *source_channels) != (rate, channels) {
            return Err("default audio format changed during startup; retry playback".into());
        }
        let src = source.clone();
        let output_name = device.description().map(|d| d.name().to_owned()).unwrap_or_else(|_| "system default output".into());
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(target_arch = "wasm32")]
        {
            let stream = play(&device, &config, &src, &Arc::new(AtomicBool::new(false)))?;
            Ok(Speaker { stop, thread: None, rate: rate as f32, channels, output_name, _stream: Some(stream) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (ready, started) = mpsc::channel();
            let flag = stop.clone();
            let thread = std::thread::Builder::new().name("audio-out".into()).spawn(move || run(device, config, src, flag, ready))
                .map_err(|e| format!("audio thread: {e}"))?;
            started.recv().map_err(|_| "audio thread ended".to_string())??;
            Ok(Speaker { stop, thread: Some(thread.thread().clone()), rate: rate as f32, channels, output_name })
        }
    })
}

/// Hold the stream; when it fails, open the default device again (with the
/// first device's rate and channels where it can) until it plays.
#[cfg(not(target_arch = "wasm32"))]
fn run(device: cpal::Device, supported: cpal::SupportedStreamConfig, src: Shared, stop: Arc<AtomicBool>, ready: mpsc::Sender<Result<(), String>>) {
    let (rate, channels) = (supported.sample_rate(), supported.channels());
    let broken = Arc::new(AtomicBool::new(false));
    let mut stream = match play(&device, &supported, &src, &broken) {
        Ok(s) => { let _ = ready.send(Ok(())); Some(s) }
        Err(e) => { let _ = ready.send(Err(e)); return; }
    };
    let mut warned = false;
    while !stop.load(Ordering::Relaxed) {
        std::thread::park_timeout(Duration::from_millis(250));
        if !broken.load(Ordering::Relaxed) { continue; }
        // Drop the dead stream first: the source is the new one's.
        if stream.take().is_some() { eprintln!("audio: output lost; waiting for a device"); }
        match reopen(rate, channels, &src, &broken) {
            Ok((name, s)) => {
                broken.store(false, Ordering::Relaxed);
                stream = Some(s);
                warned = false;
                println!("audio: playing again on {name}");
            }
            Err(e) => {
                if !warned { eprintln!("audio: {e}; trying again every second"); warned = true; }
                std::thread::park_timeout(Duration::from_secs(1));
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn reopen(rate: u32, channels: u16, src: &Shared, broken: &Arc<AtomicBool>) -> Result<(String, cpal::Stream), String> {
    with_default_output(|device| reopen_device(&device, rate, channels, src, broken))
}

#[cfg(not(target_arch = "wasm32"))]
fn reopen_device(device: &cpal::Device, rate: u32, channels: u16, src: &Shared, broken: &Arc<AtomicBool>) -> Result<(String, cpal::Stream), String> {
    let fallback = device.default_output_config().map_err(|e| format!("audio config: {e}"))?;
    // The source was built for the first device's rate and channels.
    let same = device.supported_output_configs().ok().and_then(|mut all| all.find(|c| c.channels() == channels
        && matches!(c.sample_format(), cpal::SampleFormat::F32 | cpal::SampleFormat::I16)).and_then(|c| c.try_with_sample_rate(rate)));
    let config = match same {
        Some(c) => c,
        None => {
            eprintln!("audio: the new device wants {} Hz, {} channels (was {rate} Hz, {channels})", fallback.sample_rate(), fallback.channels());
            fallback
        }
    };
    let name = device.description().map(|d| d.to_string()).unwrap_or_else(|_| "the default output".into());
    Ok((name, play(device, &config, src, broken)?))
}

fn play(device: &cpal::Device, supported: &cpal::SupportedStreamConfig, src: &Shared, broken: &Arc<AtomicBool>) -> Result<cpal::Stream, String> {
    let config = supported.config();
    let channels = config.channels as usize;
    let flag = broken.clone();
    let err = move |e: cpal::Error| {
        use cpal::ErrorKind::*;
        match e.kind() {
            Xrun => {}
            StreamInvalidated | DeviceNotAvailable | DeviceChanged => {
                if !flag.swap(true, Ordering::Relaxed) { eprintln!("audio: {e}"); }
            }
            _ => eprintln!("audio: {e}"),
        }
    };
    // Never block the audio thread: silence while the source is busy.
    let s = src.clone();
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(config, move |out: &mut [f32], _: &_| match s.try_lock() {
            Ok(mut src) => src.render(out, channels),
            Err(_) => out.fill(0.0),
        }, err, None),
        cpal::SampleFormat::I16 => {
            let mut buf = Vec::new();
            device.build_output_stream(config, move |out: &mut [i16], _: &_| {
                buf.clear();
                buf.resize(out.len(), 0.0);
                if let Ok(mut src) = s.try_lock() { src.render(&mut buf, channels); }
                for (o, s) in out.iter_mut().zip(&buf) { *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16; }
            }, err, None)
        }
        f => return Err(format!("unsupported sample format {f:?}")),
    }.map_err(|e| format!("audio stream: {e}"))?;
    stream.play().map_err(|e| format!("audio play: {e}"))?;
    Ok(stream)
}

/// The simple effects mixer on the default output with a pool of `voices`.
pub fn start(voices: usize) -> Result<(Speaker, SfxSender), String> {
    let mut tx = None;
    let speaker = open(|rate, _| { let (t, mix) = sfx(voices, rate); tx = Some(t); Box::new(mix) })?;
    Ok((speaker, tx.expect("built in open")))
}
