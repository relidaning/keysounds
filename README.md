# keysounds

Plays a mechanical-keyboard sound for every key you press, in any app (Wayland or X11).

```
./install.sh                               # fetch packs, build, link into ~/.local/bin
keysounds-toggle                           # sounds on/off, or start it (Super+Alt+K in the dotfiles)
./target/release/keysounds                 # NK Cream (default)
./target/release/keysounds -p holy-pandas -v 0.4
./target/release/keysounds --list-packs    # installed packs
./target/release/keysounds --list          # which keyboards it sees
```

The sounds are real recordings: the 18 packs bundled with [Mechvibes](https://github.com/hainguyents13/mechvibes) (MIT), in its pack format, so any Mechvibes pack from the web works too. Drop it into `~/.local/share/keysounds/packs/<name>/` or pass its directory to `-p`. `--no-release` silences the key-up sound. Modifiers (Ctrl, Shift, Alt, Super) and autorepeat never make a sound.

Needs read access to `/dev/input/event*` (be in the `input` group).

## Tray icon

keysounds puts an icon in the system tray (waybar's tray drawer here). Left click turns the sounds on/off. The right-click menu picks the sound pack (a short "hello world" sample plays straight away, so you can compare packs), the volume and the key-release sounds. The choice is remembered in `~/.config/keysounds/state.json`; `-p`/`-v` on the command line override it for that run. `--no-tray` runs without the icon.

## How it works

- `src/input.rs`: reads raw key events from the kernel through evdev, one thread per keyboard. Wayland doesn't let one app see another app's keys, so the app reads them below the display server. A watcher lists `/dev/input` every 2 s and starts a reader for any keyboard that appears later, so an unplugged or reconnected keyboard comes back by itself (it used to stay silent until a restart).
- `src/pack.rs`: loads a Mechvibes pack. Every file is decoded once at startup (symphonia: ogg/mp3/wav), mixed down to mono, resampled to the sound card's rate and peak-normalized, so all packs are equally loud. It translates key codes: Mechvibes uses libuiohook's codes, which match evdev's for the main keys but not for arrows and other extended keys.
- `src/tray.rs`: the tray icon, a StatusNotifierItem over D-Bus (`ksni`). The menu only sends messages. The main loop applies them, together with key events and SIGUSR1 from `keysounds-toggle`, and pushes the new state back to the icon.
- `src/audio.rs`: a small real-time mixer on cpal. The sound card's callback pulls new sounds (shared `Arc<[f32]>` clips, so no copying) from a channel without blocking, adds up all the active ones and soft-clips the result. Once nothing has played for 3 s the stream is paused (a running stream wakes every few milliseconds even for silence, ~1% of a core plus PipeWire's share); the next sound resumes it.

## License

MIT, see [LICENSE](LICENSE). The sound packs are not part of this repo: `fetch-packs.sh` downloads them from Mechvibes (also MIT).
