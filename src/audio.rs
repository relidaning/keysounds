//! Audio output: a tiny real-time mixer on top of cpal.
//!
//! The sound card asks for samples in small blocks from its own thread (the
//! callback below). That thread must never block, so new sounds arrive
//! through a channel it only polls with `try_recv`. Each active sound is a
//! "voice" with a play position; the callback sums all voices into the
//! output buffer and drops the ones that have finished.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// A ready-to-play mono sound. Shared, so sending one to the audio thread
/// is a pointer copy, not a copy of the samples.
pub type Clip = Arc<[f32]>;

struct Voice {
    samples: Clip,
    pos: usize,
}

pub struct Output {
    pub sample_rate: f32,
    pub tx: Sender<Clip>,
    // Dropping the stream stops playback, so it has to be kept alive.
    _stream: Stream,
}

pub fn open() -> Result<Output, Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let supported = device.default_output_config()?;
    let format = supported.sample_format();
    let mut config: cpal::StreamConfig = supported.into();
    // A small buffer keeps the delay between key press and sound short.
    // Not every backend accepts a fixed size, so fall back to the default.
    config.buffer_size = cpal::BufferSize::Fixed(256);

    let (tx, stream) = match build(&device, &config, format) {
        Ok(ok) => ok,
        Err(e) => {
            eprintln!("fixed 256-frame buffer rejected ({e}); using the default size");
            config.buffer_size = cpal::BufferSize::Default;
            build(&device, &config, format)?
        }
    };
    stream.play()?;
    Ok(Output { sample_rate: config.sample_rate as f32, tx, _stream: stream })
}

/// Builds the stream in whatever sample format the device wants, returning
/// it with the sender that feeds sounds into its callback.
fn build(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
) -> Result<(Sender<Clip>, Stream), Box<dyn std::error::Error>> {
    use cpal::SampleFormat as F;
    let (tx, rx) = channel();
    let stream = match format {
        F::F32 => stream_for::<f32>(device, config, rx)?,
        F::I16 => stream_for::<i16>(device, config, rx)?,
        F::U16 => stream_for::<u16>(device, config, rx)?,
        F::I32 => stream_for::<i32>(device, config, rx)?,
        other => return Err(format!("unsupported sample format {other:?}").into()),
    };
    Ok((tx, stream))
}

fn stream_for<S>(device: &cpal::Device, config: &cpal::StreamConfig, rx: Receiver<Clip>) -> Result<Stream, cpal::Error>
where
    S: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut voices: Vec<Voice> = Vec::with_capacity(64);

    device.build_output_stream(
        config.clone(),
        move |data: &mut [S], _| {
            while let Ok(samples) = rx.try_recv() {
                voices.push(Voice { samples, pos: 0 });
            }
            for frame in data.chunks_mut(channels) {
                let mut mix = 0.0f32;
                for v in voices.iter_mut() {
                    if let Some(s) = v.samples.get(v.pos) {
                        mix += *s;
                        v.pos += 1;
                    }
                }
                // Soft clip: fast typing can stack several sounds at once.
                let out = S::from_sample(mix.tanh());
                frame.iter_mut().for_each(|c| *c = out);
            }
            voices.retain(|v| v.pos < v.samples.len());
        },
        |e| eprintln!("audio stream error: {e}"),
        None,
    )
}
