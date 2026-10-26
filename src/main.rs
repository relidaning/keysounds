//! keysounds: plays a mechanical-keyboard sound for every key you press,
//! in any window.
//!
//!   key press ─▶ evdev reader threads ─▶ main loop picks a clip ─▶ audio callback ─▶ speakers
//!               (input.rs)              (pack.rs)                 (audio.rs)

mod audio;
mod input;
mod pack;

use std::process::exit;
use std::sync::mpsc::channel;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_PACK: &str = "nk-cream";

const USAGE: &str = "\
usage: keysounds [options]

  -p, --pack <name|dir>  sound pack (default nk-cream); see --list-packs
  -v, --volume <0..1>    loudness (default 0.6)
      --no-release       don't play a sound when keys come back up
  -P, --list-packs       list installed sound packs and exit
  -l, --list             list detected keyboards and exit
  -h, --help             show this help";

struct Args {
    pack: String,
    volume: f32,
    release: bool,
}

fn parse_args() -> Args {
    let mut args = Args { pack: DEFAULT_PACK.into(), volume: 0.6, release: true };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-p" | "--pack" => args.pack = it.next().unwrap_or_else(|| die("--pack needs a name")),
            "-v" | "--volume" => {
                let v = it.next().unwrap_or_default();
                args.volume = v
                    .parse::<f32>()
                    .ok()
                    .filter(|x| (0.0..=1.0).contains(x))
                    .unwrap_or_else(|| die("volume must be a number between 0 and 1"));
            }
            "--no-release" => args.release = false,
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

/// xorshift32, for picking between a pack's recorded variations.
fn next_rand(x: &mut u32) -> u32 {
    *x ^= *x << 13;
    *x ^= *x >> 17;
    *x ^= *x << 5;
    *x
}

fn main() {
    let args = parse_args();

    let Some(pack_dir) = pack::find(&args.pack) else {
        let dirs: Vec<String> = pack::search_dirs().iter().map(|d| d.display().to_string()).collect();
        eprintln!("keysounds: no pack '{}' in {}.\nRun fetch-packs.sh to download them.", args.pack, dirs.join(" or "));
        exit(1);
    };

    let out = audio::open().unwrap_or_else(|e| {
        eprintln!("keysounds: cannot open audio output: {e}");
        exit(1);
    });

    let pack = pack::load(&pack_dir, out.sample_rate, args.volume).unwrap_or_else(|e| {
        eprintln!("keysounds: cannot load {}: {e}", pack_dir.display());
        exit(1);
    });

    let (key_tx, key_rx) = channel();
    if input::spawn_readers(key_tx) == 0 {
        eprintln!(
            "keysounds: no readable keyboard in /dev/input.\n\
             Add yourself to the input group (sudo usermod -aG input $USER) and log in again."
        );
        exit(1);
    }
    eprintln!("pack \"{}\", volume {}. Ctrl+C to quit.", pack.name, args.volume);

    let mut rng = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1).max(1);
    for ev in key_rx {
        if !ev.pressed && !args.release {
            continue;
        }
        if let Some(clip) = pack.clip(ev.code, ev.pressed, next_rand(&mut rng)) {
            if out.tx.send(clip).is_err() {
                break;
            }
        }
    }
}
