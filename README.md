# keysounds

Plays a mechanical-keyboard sound for every key you press, in any app (Wayland or X11).

```
./install.sh                               # fetch packs, build, link into ~/.local/bin
keysounds-toggle                           # start/stop it (bound to Super+Alt+K in the dotfiles)
./target/release/keysounds                 # NK Cream (default)
./target/release/keysounds -p holy-pandas -v 0.4
./target/release/keysounds --list-packs    # installed packs
./target/release/keysounds --list          # which keyboards it sees
```

The sounds are real recordings: the 18 packs bundled with [Mechvibes](https://github.com/hainguyents13/mechvibes) (MIT), in its pack format, so any Mechvibes pack from the web works too. Drop it into `~/.local/share/keysounds/packs/<name>/` or pass its directory to `-p`. `--no-release` silences the key-up sound. Modifiers (Ctrl, Shift, Alt, Super) and autorepeat never make a sound.

Needs read access to `/dev/input/event*` (be in the `input` group).

## How it works

- `src/input.rs`: reads raw key events from the kernel through evdev, one thread per keyboard. Wayland doesn't let one app see another app's keys, so the app reads them below the display server.
- `src/pack.rs`: loads a Mechvibes pack. Every file is decoded once at startup (symphonia: ogg/mp3/wav), mixed down to mono, resampled to the sound card's rate and peak-normalized, so all packs are equally loud. It translates key codes: Mechvibes uses libuiohook's codes, which match evdev's for the main keys but not for arrows and other extended keys.
- `src/audio.rs`: a small real-time mixer on cpal. The sound card's callback pulls new sounds (shared `Arc<[f32]>` clips, so no copying) from a channel without blocking, adds up all the active ones and soft-clips the result.
