//! System tray icon (StatusNotifierItem over D-Bus, shown by waybar's tray).
//!
//! The tray only displays state and forwards clicks: every menu action
//! sends a `Msg` to the main loop, which applies it and then pushes the new
//! state back here with `Handle::update`. The main loop is the one place
//! state changes, whether a change came from the menu, the keybind
//! (SIGUSR1) or the command line.

use crate::Msg;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{MenuItem, ToolTip};
use std::sync::mpsc::Sender;

pub const VOLUMES: [f32; 5] = [0.2, 0.4, 0.6, 0.8, 1.0];

pub struct Tray {
    tx: Sender<Msg>,
    /// (directory name, display name) of every installed pack.
    pub packs: Vec<(String, String)>,
    pub pack: usize,
    pub volume: f32,
    pub muted: bool,
    pub release: bool,
}

impl Tray {
    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
    }

    fn pack_name(&self) -> &str {
        self.packs.get(self.pack).map_or("?", |p| p.1.as_str())
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "keysounds".into()
    }

    fn title(&self) -> String {
        "Key sounds".into()
    }

    // From the Flat-Remix theme; other themes fall back to a generic icon.
    fn icon_name(&self) -> String {
        if self.muted { "input-keyboard-virtual-off" } else { "input-keyboard-virtual-on" }.into()
    }

    fn tool_tip(&self) -> ToolTip {
        let state = if self.muted { "off" } else { "on" };
        ToolTip {
            title: format!("Key sounds {state}"),
            description: format!("{} · {}%", self.pack_name(), (self.volume * 100.0).round()),
            ..Default::default()
        }
    }

    /// Left click toggles the sounds; right click opens the menu.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Msg::SetMuted(!self.muted));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        // The volume radio shows the nearest step when --volume was something else.
        let vol_idx = VOLUMES
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - self.volume).abs().total_cmp(&(b.1 - self.volume).abs()))
            .map_or(0, |(i, _)| i);

        vec![
            CheckmarkItem {
                label: "Sounds on".into(),
                checked: !self.muted,
                activate: Box::new(|this: &mut Self| this.send(Msg::SetMuted(!this.muted))),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            SubMenu {
                label: format!("Sound pack: {}", self.pack_name()),
                submenu: vec![
                    RadioGroup {
                        selected: self.pack,
                        select: Box::new(|this: &mut Self, i| this.send(Msg::SetPack(i))),
                        options: self
                            .packs
                            .iter()
                            .map(|(_, name)| RadioItem { label: name.clone(), ..Default::default() })
                            .collect(),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: format!("Volume: {}%", (self.volume * 100.0).round()),
                submenu: vec![
                    RadioGroup {
                        selected: vol_idx,
                        select: Box::new(|this: &mut Self, i| this.send(Msg::SetVolume(VOLUMES[i]))),
                        options: VOLUMES
                            .iter()
                            .map(|v| RadioItem { label: format!("{}%", (v * 100.0) as u32), ..Default::default() })
                            .collect(),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Key-release sounds".into(),
                checked: self.release,
                activate: Box::new(|this: &mut Self| this.send(Msg::SetRelease(!this.release))),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Play a sample".into(),
                activate: Box::new(|this: &mut Self| this.send(Msg::Preview)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| this.send(Msg::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Registers the tray icon. With `assume_sni_available`, a tray host that
/// isn't up yet (keysounds starting before waybar) or restarts later is
/// waited for instead of being an error.
pub fn spawn(
    tx: Sender<Msg>,
    packs: Vec<(String, String)>,
    pack: usize,
    volume: f32,
    muted: bool,
    release: bool,
) -> Option<Handle<Tray>> {
    let tray = Tray { tx, packs, pack, volume, muted, release };
    match tray.assume_sni_available(true).spawn() {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("keysounds: no tray icon: {e}");
            None
        }
    }
}
