//! Speaker output for `audio::sfx`: open the default device and mix the
//! game's effects in its callback. Without a device the game stays silent.
use audio::sfx::{sfx, SfxMixer, SfxSender};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct Speaker { _stream: cpal::Stream, pub rate: f32 }

/// Open the default output with a pool of `voices`. The sender goes to the game.
pub fn start(voices: usize) -> Result<(Speaker, SfxSender), String> {
    let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
    let supported = device.default_output_config().map_err(|e| format!("audio config: {e}"))?;
    let config = supported.config();
    let rate = config.sample_rate as f32;
    let channels = config.channels as usize;
    let (tx, mut mix): (SfxSender, SfxMixer) = sfx(voices, rate);
    let err = |e| eprintln!("audio: {e}");
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(config, move |out: &mut [f32], _: &_| { out.fill(0.0); mix.render_add(out, channels); }, err, None),
        cpal::SampleFormat::I16 => {
            let mut buf = Vec::new();
            device.build_output_stream(config, move |out: &mut [i16], _: &_| {
                buf.clear();
                buf.resize(out.len(), 0.0);
                mix.render_add(&mut buf, channels);
                for (o, s) in out.iter_mut().zip(&buf) { *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16; }
            }, err, None)
        }
        f => return Err(format!("unsupported sample format {f:?}")),
    }.map_err(|e| format!("audio stream: {e}"))?;
    stream.play().map_err(|e| format!("audio play: {e}"))?;
    Ok((Speaker { _stream: stream, rate }, tx))
}
