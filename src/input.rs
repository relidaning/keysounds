//! Global keyboard capture via evdev.
//!
//! On Wayland no app can see another app's key presses through the display
//! server, so we go one level lower: the kernel exposes every input device as
//! `/dev/input/eventN`, readable by members of the `input` group. Each device
//! is read on its own thread; the threads funnel events into one channel.

use evdev::{Device, EventSummary, KeyCode};
use std::sync::mpsc::Sender;
use std::thread;

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

/// Spawns one reader thread per keyboard and returns how many were found.
/// Events are sent as any message type that can be made from a `KeyEvent`.
pub fn spawn_readers<M: From<KeyEvent> + Send + 'static>(tx: Sender<M>) -> usize {
    let mut count = 0;
    for (path, mut dev) in evdev::enumerate().filter(|(_, d)| is_keyboard(d)) {
        let tx = tx.clone();
        let name = dev.name().unwrap_or("?").to_string();
        eprintln!("listening: {} ({})", path.display(), name);
        count += 1;
        thread::spawn(move || loop {
            // fetch_events blocks until the kernel has something for us.
            let events = match dev.fetch_events() {
                Ok(ev) => ev,
                Err(e) => {
                    eprintln!("{}: {e} (device unplugged?)", path.display());
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
    count
}
