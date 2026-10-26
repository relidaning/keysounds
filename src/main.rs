//! keysounds: plays a mechanical-keyboard sound for every key you press,
//! in any window.
//!
//!   key press ─▶ evdev reader threads ─┐
//!   tray menu (tray.rs) ───────────────┼─▶ main loop ─▶ audio callback ─▶ speakers
//!   SIGUSR1 (keysounds-toggle) ────────┘   (pack.rs)    (audio.rs)

mod audio;
mod input;
mod pack;
mod tray;

use pack::Pack;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, exit};
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_PACK: &str = "nk-cream";
const DEFAULT_VOLUME: f32 = 0.6;

const USAGE: &str = "\
usage: keysounds [options]

  -p, --pack <name|dir>  sound pack (default: last one picked in the tray, else nk-cream)
  -v, --volume <0..1>    loudness (default: last one picked, else 0.6)
      --no-release       don't play a sound when keys come back up
      --no-tray          don't show a tray icon
  -P, --list-packs       list installed sound packs and exit
  -l, --list             list detected keyboards and exit
  -h, --help             show this help

Send SIGUSR1 to toggle the sounds on and off (keysounds-toggle does this).";

/// Everything the main loop reacts to.
pub enum Msg {
    Key(input::KeyEvent),
    SetPack(usize),
    SetVolume(f32),
    SetMuted(bool),
    /// From SIGUSR1, i.e. the keybind: toggles and shows a notification.
    ToggleMuted,
    SetRelease(bool),
    Preview,
    /// A sound from the preview thread; played through the main loop so
    /// it can wake a paused stream.
    Play(audio::Play),
    Quit,
}

impl From<input::KeyEvent> for Msg {
    fn from(ev: input::KeyEvent) -> Self {
        Msg::Key(ev)
    }
}

#[derive(Default)]
struct Args {
    pack: Option<String>,
    volume: Option<f32>,
    no_release: bool,
    no_tray: bool,
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-p" | "--pack" => args.pack = Some(it.next().unwrap_or_else(|| die("--pack needs a name"))),
            "-v" | "--volume" => {
                let v = it.next().unwrap_or_default();
                args.volume = Some(
                    v.parse::<f32>()
                        .ok()
                        .filter(|x| (0.0..=1.0).contains(x))
                        .unwrap_or_else(|| die("volume must be a number between 0 and 1")),
                );
            }
            "--no-release" => args.no_release = true,
            "--no-tray" => args.no_tray = true,
            "-P" | "--list-packs" => {
                for (dir, name) in pack::list() {
                    println!("{dir:26} {name}");
                }
                exit(0);
            }
            "-l" | "--list" => {
                for (path, name) in input::list_keyboards() {
                    println!("{path}\t{name}");
                }
                exit(0);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                exit(0);
            }
            other => die(&format!("unknown argument '{other}'")),
        }
    }
    args
}

fn die(msg: &str) -> ! {
    eprintln!("keysounds: {msg}\n\n{USAGE}");
    exit(2);
}

/// The pack, volume and release setting picked in the tray, kept across
/// restarts in ~/.config/keysounds/state.json.
fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("keysounds/state.json"))
}

fn load_state() -> Value {
    state_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

fn save_state(pack: &str, volume: f32, release: bool) {
    let Some(path) = state_path() else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let text = json!({ "pack": pack, "volume": (volume as f64 * 100.0).round() / 100.0, "release": release }).to_string();
    if let Err(e) = std::fs::write(&path, text) {
        eprintln!("keysounds: cannot save {}: {e}", path.display());
    }
}

/// xorshift32, for picking between a pack's recorded variations.
fn next_rand(x: &mut u32) -> u32 {
    *x ^= *x << 13;
    *x ^= *x >> 17;
    *x ^= *x << 5;
    *x
}

fn notify(text: &str) {
    let _ = Command::new("notify-send").args(["-t", "1500", "-a", "keysounds", text]).spawn();
}

/// Types "hello world" with the pack, so a newly picked pack can be heard
/// without leaving the menu.
fn play_preview(pack: Arc<Pack>, tx: Sender<Msg>, gain: f32, release: bool) {
    // evdev codes for h e l l o ␣ w o r l d
    const KEYS: [u16; 11] = [35, 18, 38, 38, 24, 57, 17, 24, 19, 38, 32];
    thread::spawn(move || {
        for (i, &code) in KEYS.iter().enumerate() {
            if let Some(c) = pack.clip(code, true, i as u32) {
                let _ = tx.send(Msg::Play((c, gain)));
            }
            thread::sleep(Duration::from_millis(55));
            if release {
                if let Some(c) = pack.clip(code, false, i as u32) {
                    let _ = tx.send(Msg::Play((c, gain)));
                }
            }
            thread::sleep(Duration::from_millis(60 + (i as u64 * 37) % 50));
        }
    });
}

fn main() {
    let args = parse_args();
    let saved = load_state();

    let mut packs = pack::list();
    let wanted = args
        .pack
        .clone()
        .or_else(|| saved["pack"].as_str().map(String::from))
        .unwrap_or_else(|| DEFAULT_PACK.into());
    let Some(pack_dir) = pack::find(&wanted) else {
        let dirs: Vec<String> = pack::search_dirs().iter().map(|d| d.display().to_string()).collect();
        eprintln!("keysounds: no pack '{wanted}' in {}.\nRun fetch-packs.sh to download them.", dirs.join(" or "));
        exit(1);
    };
    // A pack given as a path outside the pack dirs still gets a menu entry.
    let mut pack_idx = match packs.iter().position(|(d, _)| *d == wanted) {
        Some(i) => i,
        None => {
            packs.push((pack_dir.display().to_string(), wanted.clone()));
            packs.len() - 1
        }
    };

    let mut volume = args.volume.or_else(|| saved["volume"].as_f64().map(|v| v as f32)).unwrap_or(DEFAULT_VOLUME);
    let mut release = !args.no_release && saved["release"].as_bool().unwrap_or(true);
    let mut muted = false;

    let out = audio::open().unwrap_or_else(|e| {
        eprintln!("keysounds: cannot open audio output: {e}");
        exit(1);
    });

    let mut pack = Arc::new(pack::load(&pack_dir, out.sample_rate).unwrap_or_else(|e| {
        eprintln!("keysounds: cannot load {}: {e}", pack_dir.display());
        exit(1);
    }));

    let (tx, rx) = channel::<Msg>();
    if input::spawn_readers(tx.clone()) == 0 {
        eprintln!(
            "keysounds: no readable keyboard in /dev/input.\n\
             Add yourself to the input group (sudo usermod -aG input $USER) and log in again."
        );
        exit(1);
    }

    {
        let tx = tx.clone();
        let mut signals = signal_hook::iterator::Signals::new([signal_hook::consts::SIGUSR1])
            .expect("cannot install the SIGUSR1 handler");
        thread::spawn(move || {
            for _ in signals.forever() {
                let _ = tx.send(Msg::ToggleMuted);
            }
        });
    }

    let tray = if args.no_tray { None } else { tray::spawn(tx.clone(), packs.clone(), pack_idx, volume, muted, release) };
    let sync_tray = |f: &dyn Fn(&mut tray::Tray)| {
        if let Some(h) = &tray {
            h.update(|t| f(t));
        }
    };

    eprintln!("pack \"{}\", volume {volume}. Ctrl+C to quit.", pack.name);

    let mut rng = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1).max(1);
    loop {
        // Checked on every message too, so typing while muted lets it pause.
        out.pause_if_idle();
        let msg = match rx.recv_timeout(audio::IDLE_PAUSE) {
            Ok(m) => m,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match msg {
            Msg::Key(ev) => {
                if muted || (!ev.pressed && !release) {
                    continue;
                }
                if let Some(clip) = pack.clip(ev.code, ev.pressed, next_rand(&mut rng)) {
                    if out.play((clip, volume)).is_err() {
                        break;
                    }
                }
            }
            Msg::SetPack(i) => {
                let Some((dir, _)) = packs.get(i) else { continue };
                let loaded = pack::find(dir)
                    .ok_or_else(|| "pack is gone".into())
                    .and_then(|p| pack::load(&p, out.sample_rate));
                match loaded {
                    Ok(p) => {
                        pack = Arc::new(p);
                        pack_idx = i;
                        eprintln!("pack \"{}\"", pack.name);
                        save_state(dir, volume, release);
                        play_preview(pack.clone(), tx.clone(), volume, release);
                    }
                    Err(e) => {
                        eprintln!("keysounds: cannot load {dir}: {e}");
                        notify(&format!("Can't load sound pack {dir}"));
                    }
                }
                // Also on failure, so the radio button snaps back.
                sync_tray(&|t| t.pack = pack_idx);
            }
            Msg::SetVolume(v) => {
                volume = v;
                save_state(&packs[pack_idx].0, volume, release);
                sync_tray(&|t| t.volume = v);
                play_preview(pack.clone(), tx.clone(), volume, release);
            }
            Msg::SetRelease(r) => {
                release = r;
                save_state(&packs[pack_idx].0, volume, release);
                sync_tray(&|t| t.release = r);
            }
            Msg::SetMuted(m) => {
                muted = m;
                sync_tray(&|t| t.muted = m);
            }
            Msg::ToggleMuted => {
                muted = !muted;
                sync_tray(&|t| t.muted = muted);
                notify(if muted { "Key sounds off" } else { "Key sounds on" });
            }
            Msg::Play(p) => {
                let _ = out.play(p);
            }
            Msg::Preview => play_preview(pack.clone(), tx.clone(), volume, release),
            Msg::Quit => break,
        }
    }
}
