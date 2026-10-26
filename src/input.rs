//! Global keyboard capture via evdev.
//!
//! On Wayland no app can see another app's key presses through the display
//! server, so we go one level lower: the kernel exposes every input device as
//! `/dev/input/eventN`, readable by members of the `input` group. Each device
//! is read on its own thread; the threads funnel events into one channel.
//! A watcher thread picks up keyboards plugged in (or reconnected) later.

use evdev::{Device, EventSummary, KeyCode};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// How often /dev/input is listed for new devices. Listing a directory is
/// cheap; a device is only opened when its path is new.
const HOTPLUG_POLL: Duration = Duration::from_secs(2);

/// eventN path → whether it is a keyboard we're reading. A reader removes its
/// entry when the device goes away, so a reconnect at the same path is reopened.
type Known = Arc<Mutex<HashMap<PathBuf, bool>>>;

#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub code: u16,
    pub pressed: bool,
}

/// A device counts as a keyboard if it has letter keys and Enter. This skips
/// mice, power buttons, lid switches and the like, which also emit EV_KEY.
fn is_keyboard(dev: &Device) -> bool {
    dev.supported_keys().is_some_and(|keys| {
        keys.contains(KeyCode::KEY_A)
            && keys.contains(KeyCode::KEY_Z)
            && keys.contains(KeyCode::KEY_ENTER)
    })
}

/// Modifiers are held for shortcuts rather than typed, so they stay silent.
fn is_modifier(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::KEY_LEFTCTRL
            | KeyCode::KEY_RIGHTCTRL
            | KeyCode::KEY_LEFTSHIFT
            | KeyCode::KEY_RIGHTSHIFT
            | KeyCode::KEY_LEFTALT
            | KeyCode::KEY_RIGHTALT
            | KeyCode::KEY_LEFTMETA
            | KeyCode::KEY_RIGHTMETA
    )
}

pub fn list_keyboards() -> Vec<(String, String)> {
    evdev::enumerate()
        .filter(|(_, dev)| is_keyboard(dev))
        .map(|(path, dev)| {
            let name = dev.name().unwrap_or("?").to_string();
            (path.display().to_string(), name)
        })
        .collect()
}

/// Spawns one reader thread per keyboard and returns how many were found,
/// then keeps watching for keyboards that appear later.
/// Events are sent as any message type that can be made from a `KeyEvent`.
pub fn spawn_readers<M: From<KeyEvent> + Send + 'static>(tx: Sender<M>) -> usize {
    let known: Known = Arc::default();
    let mut count = 0;
    for (path, dev) in evdev::enumerate() {
        let keyboard = is_keyboard(&dev);
        known.lock().unwrap().insert(path.clone(), keyboard);
        if keyboard {
            spawn_reader(path, dev, tx.clone(), known.clone());
            count += 1;
        }
    }
    thread::spawn(move || watch_hotplug(tx, known));
    count
}

fn watch_hotplug<M: From<KeyEvent> + Send + 'static>(tx: Sender<M>, known: Known) {
    loop {
        thread::sleep(HOTPLUG_POLL);
        let Ok(entries) = std::fs::read_dir("/dev/input") else { continue };
        let present: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("event")))
            .collect();
        let new: Vec<PathBuf> = {
            let mut known = known.lock().unwrap();
            known.retain(|p, _| present.contains(p));
            present.into_iter().filter(|p| !known.contains_key(p)).collect()
        };
        for path in new {
            // A just-created node may not have its input-group permissions yet;
            // leaving it unrecorded retries it on the next pass.
            let Ok(dev) = Device::open(&path) else { continue };
            let keyboard = is_keyboard(&dev);
            known.lock().unwrap().insert(path.clone(), keyboard);
            if keyboard {
                spawn_reader(path, dev, tx.clone(), known.clone());
            }
        }
    }
}

fn spawn_reader<M: From<KeyEvent> + Send + 'static>(path: PathBuf, mut dev: Device, tx: Sender<M>, known: Known) {
    let name = dev.name().unwrap_or("?").to_string();
    eprintln!("listening: {} ({})", path.display(), name);
    thread::spawn(move || loop {
        // fetch_events blocks until the kernel has something for us.
        let events = match dev.fetch_events() {
            Ok(ev) => ev,
            Err(e) => {
                eprintln!("{}: {e} (device unplugged?)", path.display());
                known.lock().unwrap().remove(&path);
                return;
            }
        };
        for ev in events {
            // value: 1 = press, 0 = release, 2 = autorepeat. A held key
            // doesn't click again on a real keyboard, so repeats are dropped.
            if let EventSummary::Key(_, key, value) = ev.destructure() {
                if value == 2 || is_modifier(key) {
                    continue;
                }
                let event = KeyEvent { code: key.code(), pressed: value == 1 };
                if tx.send(event.into()).is_err() {
                    return;
                }
            }
        }
    });
}
