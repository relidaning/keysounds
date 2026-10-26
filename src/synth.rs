//! Procedural switch sounds.
//!
//! A keystroke on a mechanical keyboard is a few overlapping events, each
//! under ~50 ms long, and each modelled here with noise or a sine wave shaped
//! by an exponential decay:
//!
//! - **transient**: the keycap hitting the switch housing, a broadband tick
//!   of a millisecond or two (high-passed white noise);
//! - **body**: the plate and case ringing, i.e. noise through a resonant
//!   band-pass filter (its centre frequency sets how "deep" the board sounds);
//! - **thud**: the low bottom-out knock, a damped low sine;
//! - **click** (clicky switches only): the click jacket snapping, a bright
//!   band-passed burst a few milliseconds before bottom-out.
//!
//! Every key gets its own fixed pitch offset (real keys differ by position
//! on the plate), plus a little random jitter per press so repeats don't
//! sound machine-gunned.

use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Profile {
    /// Lubed linear switches on a dense case: low, round, muted.
    Thock,
    /// Click-jacket switches (Cherry MX Blue style): sharp and high.
    Clicky,
    /// Tactile switches: in between, with a short bump before bottom-out.
    Tactile,
}

impl Profile {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "thock" => Some(Self::Thock),
            "clicky" => Some(Self::Clicky),
            "tactile" => Some(Self::Tactile),
            _ => None,
        }
    }
}

/// xorshift32: tiny, fast and good enough for audio noise.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in [-1, 1).
    fn noise(&mut self) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// RBJ cookbook biquad, band-pass with constant 0 dB peak gain.
struct BandPass {
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BandPass {
    fn new(sr: f32, freq: f32, q: f32) -> Self {
        let w0 = 2.0 * PI * freq.min(sr * 0.45) / sr;
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: alpha / a0,
            b2: -alpha / a0,
            a1: -2.0 * w0.cos() / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// One-pole high-pass, used to strip the low end off the transient.
struct HighPass {
    a: f32,
    x1: f32,
    y1: f32,
}

impl HighPass {
    fn new(sr: f32, cutoff: f32) -> Self {
        let rc = 1.0 / (2.0 * PI * cutoff);
        Self { a: rc / (rc + 1.0 / sr), x1: 0.0, y1: 0.0 }
    }
    fn process(&mut self, x: f32) -> f32 {
        let y = self.a * (self.y1 + x - self.x1);
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// Big keys sit on stabilizers: lower pitched, with a small wire rattle.
fn is_wide_key(code: u16) -> bool {
    // evdev codes: 14 Backspace, 28 Enter, 42/54 Shift, 57 Space, 96 KP Enter
    matches!(code, 14 | 28 | 42 | 54 | 57 | 96)
}

/// Deterministic per-key pitch in roughly [0.92, 1.08].
fn key_pitch(code: u16) -> f32 {
    let h = (code as u32).wrapping_mul(2_654_435_761) >> 16;
    1.0 + ((h % 1000) as f32 / 1000.0 - 0.5) * 0.16
}

struct Params {
    len_ms: f32,
    transient_amp: f32,
    transient_decay: f32,
    body_freq: f32,
    body_q: f32,
    body_amp: f32,
    body_decay: f32,
    thud_freq: f32,
    thud_amp: f32,
    thud_decay: f32,
    /// (delay in ms, centre frequency, amplitude) of the click jacket/bump.
    click: Option<(f32, f32, f32)>,
}

fn params(profile: Profile, pressed: bool) -> Params {
    let mut p = match profile {
        Profile::Thock => Params {
            len_ms: 70.0,
            transient_amp: 0.35,
            transient_decay: 0.0012,
            body_freq: 650.0,
            body_q: 2.5,
            body_amp: 1.0,
            body_decay: 0.010,
            thud_freq: 140.0,
            thud_amp: 0.9,
            thud_decay: 0.012,
            click: None,
        },
        Profile::Clicky => Params {
            len_ms: 60.0,
            transient_amp: 0.5,
            transient_decay: 0.0010,
            body_freq: 2400.0,
            body_q: 4.0,
            body_amp: 0.7,
            body_decay: 0.006,
            thud_freq: 220.0,
            thud_amp: 0.35,
            thud_decay: 0.006,
            click: Some((0.0, 4200.0, 1.2)),
        },
        Profile::Tactile => Params {
            len_ms: 60.0,
            transient_amp: 0.45,
            transient_decay: 0.0010,
            body_freq: 1300.0,
            body_q: 3.0,
            body_amp: 0.9,
            body_decay: 0.008,
            thud_freq: 180.0,
            thud_amp: 0.6,
            thud_decay: 0.009,
            click: Some((0.0, 2000.0, 0.35)),
        },
    };
    if !pressed {
        // Release: the stem springs back up to the top housing. No bottom-out
        // thud and no click, just a shorter, higher, quieter tap.
        p.len_ms *= 0.6;
        p.transient_amp *= 0.5;
        p.body_freq *= 1.5;
        p.body_amp *= 0.35;
        p.body_decay *= 0.6;
        p.thud_amp = 0.0;
        p.click = None;
    }
    p
}

/// Renders one keystroke sound, mono, peak-normalized to `gain`.
pub fn render(sr: f32, profile: Profile, ev_code: u16, pressed: bool, gain: f32, rng: &mut Rng) -> Vec<f32> {
    let p = params(profile, pressed);
    let wide = is_wide_key(ev_code);
    let jitter = 1.0 + rng.noise() * 0.03;
    let pitch = key_pitch(ev_code) * jitter * if wide { 0.75 } else { 1.0 };

    let n = (sr * p.len_ms / 1000.0) as usize;
    let mut out = vec![0.0f32; n];

    let mut hp = HighPass::new(sr, 3000.0);
    let mut body = BandPass::new(sr, p.body_freq * pitch, p.body_q);
    let mut click_bp = p.click.map(|(_, f, _)| BandPass::new(sr, f * pitch, 3.0));
    // Stabilizer rattle: a second, quieter body ring ~12 ms later.
    let mut rattle = wide.then(|| BandPass::new(sr, p.body_freq * pitch * 1.8, 6.0));
    let rattle_start = (sr * 0.012) as usize;

    // In clicky/tactile switches the event happens before bottom-out, so the
    // rest of the sound is delayed past it.
    let click_ms = match p.click {
        Some(_) if pressed => 6.0 + rng.noise() * 0.8,
        _ => 0.0,
    };
    let bottom = (sr * click_ms / 1000.0) as usize;

    for (i, s) in out.iter_mut().enumerate() {
        let mut v = 0.0;

        if let (Some(bp), Some((_, _, amp))) = (click_bp.as_mut(), p.click) {
            let t = i as f32 / sr;
            v += bp.process(rng.noise()) * amp * (-t / 0.0015).exp();
        }

        if i >= bottom {
            let t = (i - bottom) as f32 / sr;
            v += hp.process(rng.noise()) * p.transient_amp * (-t / p.transient_decay).exp();
            v += body.process(rng.noise()) * p.body_amp * (-t / p.body_decay).exp();
            v += (2.0 * PI * p.thud_freq * pitch * t).sin() * p.thud_amp * (-t / p.thud_decay).exp();
        }

        if let Some(r) = rattle.as_mut() {
            if i >= bottom + rattle_start {
                let t = (i - bottom - rattle_start) as f32 / sr;
                v += r.process(rng.noise()) * 0.25 * (-t / 0.004).exp();
            }
        }

        *s = v;
    }

    // Short fade-in avoids a DC step "pop" at the very first sample.
    for (i, s) in out.iter_mut().take(16).enumerate() {
        *s *= i as f32 / 16.0;
    }

    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        let level = if pressed { gain } else { gain * 0.55 };
        let k = level / peak;
        out.iter_mut().for_each(|s| *s *= k);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_finite_audible_and_within_gain() {
        let mut rng = Rng::new(42);
        for profile in [Profile::Thock, Profile::Tactile, Profile::Clicky] {
            for code in [30u16, 57, 28, 14, 2] {
                for pressed in [true, false] {
                    let s = render(48_000.0, profile, code, pressed, 0.5, &mut rng);
                    assert!(!s.is_empty());
                    assert!(s.iter().all(|x| x.is_finite()));
                    let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                    assert!(peak > 0.1 && peak <= 0.5 + 1e-4, "{profile:?} {code} peak {peak}");
                }
            }
        }
    }
}
