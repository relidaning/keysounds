//! Mechvibes-format sound packs: real recordings of keyboards.
//!
//! A pack is a directory with a `config.json` and audio files. Two layouts:
//!
//! - `"key_define_type": "single"`: one long recording (`sound`) of every key
//!   pressed in turn; `defines` maps a key code to `[start_ms, length_ms]`.
//! - `"key_define_type": "multi"`: `defines` maps a key code to its own file.
//!   Newer (version 2) packs also give a generic press sound in `sound`,
//!   with `{0-4}` meaning "pick one of five variations at random", a generic
//!   release sound in `soundup`, and per-key releases as `"<code>-up"`.
//!
//! Key codes are libuiohook's (what Mechvibes listens with), which equal
//! Linux evdev codes for the main block; extended keys are translated below.

use crate::audio::Clip;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct Pack {
    pub name: String,
    press: HashMap<u16, Vec<Clip>>,
    release: HashMap<u16, Clip>,
    generic_press: Vec<Clip>,
    generic_release: Option<Clip>,
    /// Every distinct press clip, for keys the pack doesn't define.
    all_press: Vec<Clip>,
}

impl Pack {
    /// The sound for a key event, or None for silence. `roll` is a random
    /// number used to pick between variations.
    pub fn clip(&self, evdev_code: u16, pressed: bool, roll: u32) -> Option<Clip> {
        let code = to_uiohook(evdev_code);
        if pressed {
            let pick = |v: &Vec<Clip>| (!v.is_empty()).then(|| v[roll as usize % v.len()].clone());
            self.press
                .get(&code)
                .and_then(pick)
                .or_else(|| pick(&self.generic_press))
                // No generic sound: give each unknown key a fixed sound
                // borrowed from one of the recorded keys.
                .or_else(|| {
                    let n = self.all_press.len();
                    (n > 0).then(|| self.all_press[(code as usize).wrapping_mul(2_654_435_761) % n].clone())
                })
        } else {
            self.release.get(&code).or(self.generic_release.as_ref()).cloned()
        }
    }
}

/// evdev key code → libuiohook key code. Codes 1-88 (Esc through F12) are
/// the same PC scan codes in both; extended keys are 0x0Exx in libuiohook,
/// and arrows 0xE0xx in the version Mechvibes packs were made with.
fn to_uiohook(code: u16) -> u16 {
    match code {
        96 => 3612,   // KP Enter
        97 => 3613,   // Right Ctrl
        98 => 3637,   // KP /
        99 => 3639,   // Print Screen
        100 => 3640,  // Right Alt
        119 => 3653,  // Pause
        102 => 3655,  // Home
        104 => 3657,  // Page Up
        107 => 3663,  // End
        109 => 3665,  // Page Down
        110 => 3666,  // Insert
        111 => 3667,  // Delete
        125 => 3675,  // Left Super
        126 => 3676,  // Right Super
        127 => 3677,  // Menu
        103 => 57416, // Up
        105 => 57419, // Left
        106 => 57421, // Right
        108 => 57424, // Down
        c => c,
    }
}

/// Where packs are looked up, in order.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".local/share/keysounds/packs"));
    }
    dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("packs"));
    dirs
}

/// (directory name, display name) of every installed pack.
pub fn list() -> Vec<(String, String)> {
    let mut packs = Vec::new();
    for dir in search_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let cfg = e.path().join("config.json");
            let Ok(text) = std::fs::read_to_string(&cfg) else { continue };
            let name = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["name"].as_str().map(String::from))
                .unwrap_or_default();
            packs.push((e.file_name().to_string_lossy().into_owned(), name));
        }
    }
    packs.sort();
    packs.dedup_by(|a, b| a.0 == b.0);
    packs
}

/// Finds a pack by directory name, or takes `name` as a path to one.
pub fn find(name: &str) -> Option<PathBuf> {
    let direct = Path::new(name);
    if direct.join("config.json").is_file() {
        return Some(direct.to_path_buf());
    }
    search_dirs().into_iter().map(|d| d.join(name)).find(|p| p.join("config.json").is_file())
}

/// Loads a pack, resampled to `sample_rate` and scaled so its loudest sound
/// peaks at `volume`. Packs are recorded at very different levels; this
/// makes them all equally loud.
pub fn load(dir: &Path, sample_rate: f32, volume: f32) -> Result<Pack> {
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
    let name = cfg["name"].as_str().unwrap_or("?").to_string();
    let defines = cfg["defines"].as_object().ok_or("config.json has no \"defines\"")?;
    let single = cfg["key_define_type"].as_str() == Some("single");

    // Decoded files, by name. Many keys share one file in multi packs.
    let mut files: HashMap<String, Arc<Vec<f32>>> = HashMap::new();
    let mut decode = |file: &str| -> Result<Arc<Vec<f32>>> {
        if let Some(d) = files.get(file) {
            return Ok(d.clone());
        }
        let d = Arc::new(decode_mono(&dir.join(file), sample_rate)?);
        files.insert(file.to_string(), d.clone());
        Ok(d)
    };

    let mut press: HashMap<u16, Vec<Vec<f32>>> = HashMap::new();
    let mut release: HashMap<u16, Vec<f32>> = HashMap::new();
    let mut generic_press = Vec::new();
    let mut generic_release = None;

    if single {
        let sound = cfg["sound"].as_str().ok_or("single pack without \"sound\"")?;
        let all = decode(sound)?;
        for (key, val) in defines {
            let Some(arr) = val.as_array() else { continue };
            let (Some(start), Some(len)) = (arr.first().and_then(Value::as_f64), arr.get(1).and_then(Value::as_f64))
            else {
                continue;
            };
            let a = ((start / 1000.0) * sample_rate as f64) as usize;
            let b = (a + ((len / 1000.0) * sample_rate as f64) as usize).min(all.len());
            if a >= b {
                continue;
            }
            insert(key, all[a..b].to_vec(), &mut press, &mut release);
        }
    } else {
        for (key, val) in defines {
            let Some(file) = val.as_str() else { continue };
            match decode(file) {
                Ok(d) => insert(key, (*d).clone(), &mut press, &mut release),
                Err(e) => eprintln!("{name}: skipping {file}: {e}"),
            }
        }
        // In version 1 multi packs `sound` is a leftover, often missing.
        if let Some(pattern) = cfg["sound"].as_str() {
            for file in expand(pattern) {
                if let Ok(d) = decode(&file) {
                    generic_press.push((*d).clone());
                }
            }
        }
        if let Some(file) = cfg["soundup"].as_str() {
            generic_release = decode(file).ok().map(|d| (*d).clone());
        }
    }

    // One gain for the whole pack keeps the recorded balance between keys.
    let peak = press
        .values()
        .flatten()
        .chain(&generic_press)
        .flat_map(|c| c.iter())
        .fold(0.0f32, |m, s| m.max(s.abs()));
    if peak == 0.0 {
        return Err("pack has no audible press sounds".into());
    }
    let gain = volume / peak;
    let clip = |mut v: Vec<f32>| -> Clip {
        v.iter_mut().for_each(|s| *s *= gain);
        fade_out(&mut v, sample_rate);
        v.into()
    };

    let press: HashMap<u16, Vec<Clip>> =
        press.into_iter().map(|(k, v)| (k, v.into_iter().map(clip).collect())).collect();
    let mut all_press: Vec<Clip> = press.values().flatten().cloned().collect();
    let generic_press: Vec<Clip> = generic_press.into_iter().map(clip).collect();
    all_press.extend(generic_press.iter().cloned());

    Ok(Pack {
        name,
        release: release.into_iter().map(|(k, v)| (k, clip(v))).collect(),
        generic_release: generic_release.map(clip),
        press,
        generic_press,
        all_press,
    })
}

/// Files `"14"` under press and `"14-up"` under release.
fn insert(key: &str, clip: Vec<f32>, press: &mut HashMap<u16, Vec<Vec<f32>>>, release: &mut HashMap<u16, Vec<f32>>) {
    let (num, up) = match key.strip_suffix("-up") {
        Some(n) => (n, true),
        None => (key, false),
    };
    let Ok(code) = num.parse::<u16>() else { return };
    if up {
        release.insert(code, clip);
    } else {
        press.entry(code).or_default().push(clip);
    }
}

/// `"press/GENERIC_R{0-4}.mp3"` → `press/GENERIC_R0.mp3` … `press/GENERIC_R4.mp3`.
fn expand(pattern: &str) -> Vec<String> {
    let range = pattern.find('{').zip(pattern.find('}')).and_then(|(a, b)| {
        let (lo, hi) = pattern[a + 1..b].split_once('-')?;
        Some((a, b, lo.parse::<u32>().ok()?, hi.parse::<u32>().ok()?))
    });
    match range {
        Some((a, b, lo, hi)) => (lo..=hi).map(|i| format!("{}{i}{}", &pattern[..a], &pattern[b + 1..])).collect(),
        None => vec![pattern.to_string()],
    }
}

/// Slices of a long recording end wherever the offset says, often mid-wave;
/// a 3 ms fade avoids an audible click at the cut.
fn fade_out(v: &mut [f32], sample_rate: f32) {
    let n = ((sample_rate * 0.003) as usize).min(v.len());
    let len = v.len();
    for (i, s) in v[len - n..].iter_mut().enumerate() {
        *s *= 1.0 - i as f32 / n as f32;
    }
}

/// Decodes any supported audio file to mono f32 at `target_rate`.
fn decode_mono(path: &Path, target_rate: f32) -> Result<Vec<f32>> {
    let mss = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio track")?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.ok_or("unknown sample rate")? as f32;
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut mono = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue, // a corrupt frame: skip it
            Err(e) => return Err(e.into()),
        };
        let channels = decoded.spec().channels.count();
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buf.copy_interleaved_ref(decoded);
        mono.extend(buf.samples().chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32));
    }
    Ok(resample(&mono, rate, target_rate))
}

/// Linear-interpolation resampling. Crude for music, inaudible on clicks.
fn resample(input: &[f32], from: f32, to: f32) -> Vec<f32> {
    if (from - to).abs() < 1.0 || input.is_empty() {
        return input.to_vec();
    }
    let step = from / to;
    let n = (input.len() as f32 / step) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f32 * step;
            let j = pos as usize;
            let frac = pos - j as f32;
            let a = input[j];
            let b = *input.get(j + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_variation_patterns() {
        assert_eq!(expand("p/G_R{0-2}.mp3"), ["p/G_R0.mp3", "p/G_R1.mp3", "p/G_R2.mp3"]);
        assert_eq!(expand("sound.ogg"), ["sound.ogg"]);
    }

    #[test]
    fn every_installed_pack_loads() {
        let packs = list();
        for (dir, _) in &packs {
            let pack = load(&find(dir).unwrap(), 48_000.0, 0.6).unwrap_or_else(|e| panic!("{dir}: {e}"));
            for code in [30u16, 57, 28, 14, 103, 200] {
                let c = pack.clip(code, true, 7).unwrap_or_else(|| panic!("{dir}: no sound for {code}"));
                assert!(!c.is_empty() && c.iter().all(|s| s.is_finite() && s.abs() <= 0.6 + 1e-4));
            }
        }
    }
}
