# keysounds

Plays a mechanical-keyboard sound for every key you press, in any app (Wayland or X11).

```
cargo build --release
./target/release/keysounds                 # thock (default)
./target/release/keysounds -p clicky -v 0.4
./target/release/keysounds --list          # which keyboards it sees
```

Profiles: `thock`, `tactile`, `clicky`. `--no-release` silences the key-up sound.

Needs read access to `/dev/input/event*` (be in the `input` group).

## How it works

- `src/input.rs`: reads raw key events from the kernel through evdev, one thread per keyboard. Wayland doesn't let one app see another app's keys, so the app reads them below the display server.
- `src/synth.rs`: makes each keystroke sound from scratch, with no sample files: a noise tick for the keycap, band-passed noise for the plate/case ring, a damped low sine for the bottom-out thud, and a bright burst for click jackets. Each key has its own pitch, and each press gets a little random jitter.
- `src/audio.rs`: a small real-time mixer on cpal. The sound card's callback pulls new sounds from a channel without blocking, adds up all the active ones and soft-clips the result.
