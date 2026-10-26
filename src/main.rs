//! keysounds: plays a mechanical-keyboard sound for every key you press,
//! in any window.
//!
//!   key press ─▶ evdev reader threads ─▶ synth thread ─▶ audio callback ─▶ speakers
//!               (input.rs)              (synth.rs)      (audio.rs)

mod audio;
mod input;
mod synth;

use std::process::exit;
use std::sync::mpsc::channel;
use std::time::{SystemTime, UNIX_EPOCH};
use synth::{Profile, Rng};

const USAGE: &str = "\
usage: keysounds [options]

  -p, --profile <name>   thock (default), tactile, clicky
  -v, --volume <0..1>    loudness (default 0.5)
      --no-release       don't play a sound when keys come back up
  -l, --list             list detected keyboards and exit
  -h, --help             show this help";

struct Args {
    profile: Profile,
    volume: f32,
    release: bool,
}

fn parse_args() -> Args {
    let mut args = Args { profile: Profile::Thock, volume: 0.5, release: true };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-p" | "--profile" => {
                let v = it.next().unwrap_or_default();
                args.profile = Profile::parse(&v).unwrap_or_else(|| die(&format!("unknown profile '{v}'")));
            }
            "-v" | "--volume" => {
                let v = it.next().unwrap_or_default();
                args.volume = v
                    .parse::<f32>()
                    .ok()
                    .filter(|x| (0.0..=1.0).contains(x))
                    .unwrap_or_else(|| die("volume must be a number between 0 and 1"));
            }
            "--no-release" => args.release = false,
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

fn main() {
    let args = parse_args();

    let out = audio::open().unwrap_or_else(|e| {
        eprintln!("keysounds: cannot open audio output: {e}");
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
    eprintln!("profile {:?}, volume {}. Ctrl+C to quit.", args.profile, args.volume);

    let seed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1);
    let mut rng = Rng::new(seed);

    // Rendering a 70 ms sound takes microseconds, so each keystroke gets a
    // freshly synthesized, slightly different sound instead of a sample.
    for ev in key_rx {
        if !ev.pressed && !args.release {
            continue;
        }
        let sound = synth::render(out.sample_rate, args.profile, ev.code, ev.pressed, args.volume, &mut rng);
        if out.tx.send(sound).is_err() {
            break;
        }
    }
}
