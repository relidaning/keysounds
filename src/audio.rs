//! Audio output: a tiny real-time mixer on top of cpal.
//!
//! The sound card asks for samples in small blocks from its own thread (the
//! callback below). That thread must never block, so new sounds arrive
//! through a channel it only polls with `try_recv`. Each active sound is a
//! "voice" with a play position; the callback sums all voices into the
//! output buffer and drops the ones that have finished.
//!
//! A running stream wakes up every few milliseconds even when it only plays
//! silence (about 1% of a core here, and it keeps PipeWire busy too), so
//! the stream is paused once nothing has played for a while and resumed by
//! the next sound.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// How long the stream keeps running after the last sound was sent.
pub const IDLE_PAUSE: Duration = Duration::from_secs(3);

/// A ready-to-play mono sound. Shared, so sending one to the audio thread
/// is a pointer copy, not a copy of the samples.
pub type Clip = Arc<[f32]>;

/// A clip and the gain to play it at. The volume is applied while mixing,
/// so changing it takes effect on the next key without reloading the pack.
pub type Play = (Clip, f32);

struct Voice {
    samples: Clip,
    gain: f32,
    pos: usize,
}

pub struct Output {
    pub sample_rate: f32,
    tx: Sender<Play>,
    // Dropping the stream stops playback, so it has to be kept alive.
    stream: Stream,
    /// Set by the callback while no voice is left to play.
    silent: Arc<AtomicBool>,
    playing: Cell<bool>,
    last_sent: Cell<Instant>,
}

impl Output {
    /// Queues a sound, resuming the stream first if it was paused.
    pub fn play(&self, play: Play) -> Result<(), ()> {
        self.tx.send(play).map_err(|_| ())?;
        self.last_sent.set(Instant::now());
        if !self.playing.get() {
            match self.stream.play() {
                Ok(()) => self.playing.set(true),
                Err(e) => eprintln!("cannot resume audio stream: {e}"),
            }
        }
        Ok(())
    }

    /// Pauses the stream if nothing was sent for `IDLE_PAUSE` and the last
    /// sound has finished. A device that can't pause just keeps running.
    pub fn pause_if_idle(&self) {
        if !self.playing.get() || self.last_sent.get().elapsed() < IDLE_PAUSE || !self.silent.load(Ordering::Relaxed) {
            return;
        }
        match self.stream.pause() {
            Ok(()) => self.playing.set(false),
            Err(e) => {
                eprintln!("cannot pause audio stream ({e}); leaving it running");
                // Don't retry every few seconds.
                self.last_sent.set(Instant::now() + Duration::from_secs(1 << 20));
            }
        }
    }
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

    let silent = Arc::new(AtomicBool::new(true));
    let (tx, stream) = match build(&device, &config, format, &silent) {
        Ok(ok) => ok,
        Err(e) => {
            eprintln!("fixed 256-frame buffer rejected ({e}); using the default size");
            config.buffer_size = cpal::BufferSize::Default;
            build(&device, &config, format, &silent)?
        }
    };
    stream.play()?;
    Ok(Output {
        sample_rate: config.sample_rate as f32,
        tx,
        stream,
        silent,
        playing: Cell::new(true),
        last_sent: Cell::new(Instant::now()),
    })
}

/// Builds the stream in whatever sample format the device wants, returning
/// it with the sender that feeds sounds into its callback.
fn build(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    silent: &Arc<AtomicBool>,
) -> Result<(Sender<Play>, Stream), Box<dyn std::error::Error>> {
    use cpal::SampleFormat as F;
    let (tx, rx) = channel();
    let stream = match format {
        F::F32 => stream_for::<f32>(device, config, rx, silent.clone())?,
        F::I16 => stream_for::<i16>(device, config, rx, silent.clone())?,
        F::U16 => stream_for::<u16>(device, config, rx, silent.clone())?,
        F::I32 => stream_for::<i32>(device, config, rx, silent.clone())?,
        other => return Err(format!("unsupported sample format {other:?}").into()),
    };
    Ok((tx, stream))
}

fn stream_for<S>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    rx: Receiver<Play>,
    silent: Arc<AtomicBool>,
) -> Result<Stream, cpal::Error>
where
    S: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut voices: Vec<Voice> = Vec::with_capacity(64);

    device.build_output_stream(
        config.clone(),
        move |data: &mut [S], _| {
            while let Ok((samples, gain)) = rx.try_recv() {
                voices.push(Voice { samples, gain, pos: 0 });
            }
            for frame in data.chunks_mut(channels) {
                let mut mix = 0.0f32;
                for v in voices.iter_mut() {
                    if let Some(s) = v.samples.get(v.pos) {
                        mix += *s * v.gain;
                        v.pos += 1;
                    }
                }
                // Soft clip: fast typing can stack several sounds at once.
                let out = S::from_sample(mix.tanh());
                frame.iter_mut().for_each(|c| *c = out);
            }
            voices.retain(|v| v.pos < v.samples.len());
            silent.store(voices.is_empty(), Ordering::Relaxed);
        },
        |e| eprintln!("audio stream error: {e}"),
        None,
    )
}
