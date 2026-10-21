//! MN3009 BBD stereo chorus.
//!
//! Ported from the Ultramaster KR-106 `KR106Chorus.h` / `BBDFilter.h`.
//! Two BBD lines driven in antiphase by one LFO; each output is an
//! inverting summer of dry + one wet tap. The BBD is a Hermite-interpolated
//! fractional delay (the Juno's ~40 kHz clock puts BBD aliasing above audio
//! Nyquist), with the matched pre/post filters, input saturation, clock-
//! rate-dependent charge-transfer loss, leakage noise, clock-reset clicks
//! and the per-channel ~30 Hz output-stage ring measured on hardware.

use std::f32::consts::PI;

use super::noise::{FloorNoise, RailRipple, bipolar, levels};

/// Ring buffer length: >= 20 ms at 192 kHz plus Hermite margin.
const BUF_LEN: usize = 4096;
const BUF_MASK: usize = BUF_LEN - 1;
const BBD_STAGES: f32 = 256.0;

/// Center delay, measured from hardware click timing.
const CENTER_DELAY_MS: f32 = 3.30;
const MIN_DELAY_MS: f32 = 0.1;
const MIN_CLOCK_HZ: f32 = 5000.0;

/// Juno-106 chorus LFO (jack board IC1): a Schmitt trigger + integrator
/// triangle. The CHORUS I/II line only switches the integrator input
/// divider through JFET Tr1 (R4 680K vs 680K + R3 2.2M), so the two modes
/// differ in rate only: nominal 0.55 / 0.90 Hz (ratio 1.62). Chorus I is
/// the KR-106's hardware measurement; II applies the same component
/// correction. The triangle amplitude is fixed by R7/R6, hence the same
/// delay swing in both modes.
const CHORUS_I_RATE: f32 = 0.514;
const CHORUS_II_RATE: f32 = 0.835;
const CHORUS_I_DEPTH_MS: f32 = 2.13;
const CHORUS_II_DEPTH_MS: f32 = CHORUS_I_DEPTH_MS;
/// I+II does not exist on the Juno-106 (two control lines only); this is
/// the Juno-6/60 vibrato-like mode, kept as an extra.
const CHORUS_I_II_RATE: f32 = 7.85;
const CHORUS_I_II_DEPTH_MS: f32 = 0.236;

/// IC6 summer gains solved from hardware envelope extremes.
const DRY_GAIN: f32 = 0.863;
const WET_GAIN: f32 = 1.257;

const CTE_LOSS_COEFF: f32 = 4468.0;
const INV_CLOCK_CENTER: f32 = 1.0 / 40_000.0;
const LEAKAGE_GAIN: f32 = 8.8e-3;
const LEAK_MIN_FRAC: f32 = 0.0126;
/// Per-BBD clock / gain tolerance (two MN3009s are never identical).
const CLOCK_TRIM: f32 = 0.015;
const GAIN_TRIM: f32 = 0.04;
const CLICK_GAIN: f32 = 0.11;
const SLOW_CLICK_GAIN: f32 = 0.022;
const RING_HZ: f32 = 30.0;
const RING_Q: f32 = 18.0;
const RING_GAIN: f32 = 0.06;
const FADE_MS: f32 = 5.0;
/// Default BBD saturation drive (user drive 50/100 -> 0.48 * 0.25).
const SAT_DRIVE: f32 = 0.12;

/// Pre/post filter: Butterworth biquad at 9661 Hz + 1-pole at 20 kHz
/// (fit to ngspice of the 2SA1015 emitter-follower stages).
#[derive(Clone, Default)]
struct BbdFilter {
    ic1: f32,
    ic2: f32,
    pole: f32,
    g_bq: f32,
    a1_bq: f32,
    g_p: f32,
}

impl BbdFilter {
    fn init(&mut self, sample_rate: f32) {
        let fc = 9661.0_f32.min(sample_rate * 0.45);
        self.g_bq = (PI * fc / sample_rate).tan();
        self.a1_bq = 1.0 / (1.0 + self.g_bq / 0.7071 + self.g_bq * self.g_bq);
        let fp = 20000.0_f32.min(sample_rate * 0.45);
        self.g_p = (PI * fp / sample_rate).tan();
        self.ic1 = 0.0;
        self.ic2 = 0.0;
        self.pole = 0.0;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let v3 = input - self.ic2;
        let v1 = self.a1_bq * self.ic1 + self.a1_bq * self.g_bq * v3;
        let v2 = self.ic2 + self.g_bq * v1;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        let v = (v2 - self.pole) * self.g_p / (1.0 + self.g_p);
        let lp = self.pole + v;
        self.pole = lp + v;
        lp
    }
}

#[derive(Clone)]
struct BbdLine {
    buf: Box<[f32; BUF_LEN]>,
    wpos: usize,
    pre: BbdFilter,
    post: BbdFilter,
}

impl Default for BbdLine {
    fn default() -> Self {
        Self { buf: Box::new([0.0; BUF_LEN]), wpos: 0, pre: BbdFilter::default(), post: BbdFilter::default() }
    }
}

#[inline]
fn hermite(frac: f32, y0: f32, y1: f32, y2: f32, y3: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * frac + c2) * frac + c1) * frac + y1
}

impl BbdLine {
    fn init(&mut self, sample_rate: f32) {
        self.buf.fill(0.0);
        self.wpos = 0;
        self.pre.init(sample_rate);
        self.post.init(sample_rate);
    }

    #[inline]
    fn process(&mut self, input: f32, delay_samples: f32, injected: f32) -> f32 {
        let filtered = self.pre.process(input + injected);
        // MN3009 headroom: soft tanh knee, only reached by hot patches.
        let sat = (filtered * SAT_DRIVE).tanh() / SAT_DRIVE;
        self.buf[self.wpos] = sat;

        let mut rpos = self.wpos as f32 - delay_samples;
        if rpos < 0.0 {
            rpos += BUF_LEN as f32;
        }
        let i1 = rpos as usize;
        let frac = rpos - i1 as f32;
        let b = &self.buf;
        let wet = hermite(
            frac,
            b[(i1 + BUF_LEN - 1) & BUF_MASK],
            b[i1 & BUF_MASK],
            b[(i1 + 1) & BUF_MASK],
            b[(i1 + 2) & BUF_MASK],
        );
        self.wpos = (self.wpos + 1) & BUF_MASK;
        self.post.process(wet)
    }
}

/// Biphasic clock-reset glitch at an LFO turnaround.
#[derive(Clone, Default)]
struct BbdClick {
    counter: i32,
    duration: i32,
    was_in_zone: bool,
}

impl BbdClick {
    fn init(&mut self, sample_rate: f32) {
        self.duration = (180.0 * 0.001 * sample_rate) as i32;
        self.counter = -1;
        self.was_in_zone = false;
    }

    fn suppress(&mut self) {
        self.counter = -1;
        self.was_in_zone = true;
    }

    #[inline]
    fn process(&mut self, lfo_for_line: f32) -> f32 {
        let in_zone = lfo_for_line > 0.95;
        if in_zone && !self.was_in_zone {
            self.counter = 0;
        }
        self.was_in_zone = in_zone;
        if self.counter < 0 || self.counter >= self.duration {
            self.counter = -1;
            return 0.0;
        }
        let t = self.counter as f32 / self.duration as f32;
        self.counter += 1;
        const ASYM: f32 = 0.20;
        if t < ASYM {
            let u = t / ASYM;
            -(-4.0 * u).exp() * (PI * u).sin()
        } else {
            let u = (t - ASYM) / (1.0 - ASYM);
            0.85 * (-8.0 * u).exp() * (PI * u).sin()
        }
    }
}

/// Band-passed (800 Hz HPF + 4 kHz LPF) BBD leakage noise.
#[derive(Clone)]
struct LeakNoise {
    seed: u32,
    hp_state: f32,
    hp_coeff: f32,
    lp_state: f32,
    lp_coeff: f32,
}

impl LeakNoise {
    fn new(seed: u32) -> Self {
        Self { seed, hp_state: 0.0, hp_coeff: 0.0, lp_state: 0.0, lp_coeff: 0.0 }
    }

    fn init(&mut self, sample_rate: f32) {
        self.hp_coeff = 1.0 - (-2.0 * PI * 800.0 / sample_rate).exp();
        self.lp_coeff = 1.0 - (-2.0 * PI * 4000.0 / sample_rate).exp();
    }

    #[inline]
    fn process(&mut self) -> f32 {
        let white = bipolar(&mut self.seed);
        self.hp_state += self.hp_coeff * (white - self.hp_state);
        let hp = white - self.hp_state;
        self.lp_state += self.lp_coeff * (hp - self.lp_state);
        self.lp_state
    }
}

/// ~30 Hz, Q 18 output-stage resonance excited by each click
/// (Chamberlin SVF, lowpass out).
#[derive(Clone, Default)]
struct ClickRing {
    f: f32,
    damp: f32,
    low: f32,
    band: f32,
}

impl ClickRing {
    fn init(&mut self, sample_rate: f32) {
        self.f = 2.0 * (PI * RING_HZ / sample_rate).sin();
        self.damp = 1.0 / RING_Q;
        self.reset();
    }

    fn reset(&mut self) {
        self.low = 0.0;
        self.band = 0.0;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        self.low += self.f * self.band;
        let high = input - self.low - self.damp * self.band;
        self.band += self.f * high;
        self.low
    }
}

/// Chorus mode: 0 off, 1 = I, 2 = II, 3 = I+II.
#[derive(Clone)]
pub struct Chorus {
    line0: BbdLine,
    line1: BbdLine,
    lfo_phase: f32,
    lfo_inc: f32,
    use_sine: bool,
    click0: BbdClick,
    click1: BbdClick,
    slow_click0: BbdClick,
    slow_click1: BbdClick,
    leak0: LeakNoise,
    leak1: LeakNoise,
    ring0: ClickRing,
    ring1: ClickRing,
    wet_pink0: FloorNoise,
    wet_pink1: FloorNoise,
    wet_ripple: RailRipple,
    mode: u32,
    pending_mode: u32,
    sample_rate: f32,
    fade: f32,
    fade_target: f32,
    fade_inc: f32,
    delay_depth: f32,
    target_delay_depth: f32,
    gain_mod_scale: f32,
    /// Analog noise multiplier (wet broadband, leakage, clicks, ripple).
    pub noise_mul: f32,
    /// BBD hiss multiplier (wet broadband + leakage only), on top of
    /// `noise_mul`. Clicks and ripple stay under `noise_mul`.
    pub hiss_mul: f32,
}

impl Default for Chorus {
    fn default() -> Self {
        Self {
            line0: BbdLine::default(),
            line1: BbdLine::default(),
            lfo_phase: 0.0,
            lfo_inc: 0.0,
            use_sine: false,
            click0: BbdClick::default(),
            click1: BbdClick::default(),
            slow_click0: BbdClick::default(),
            slow_click1: BbdClick::default(),
            leak0: LeakNoise::new(0xDEAD_BEEF),
            leak1: LeakNoise::new(0x1BAD_B002),
            ring0: ClickRing::default(),
            ring1: ClickRing::default(),
            wet_pink0: FloorNoise::new(0x1234_5678),
            wet_pink1: FloorNoise::new(0x8765_4321),
            wet_ripple: RailRipple::default(),
            mode: 0,
            pending_mode: 0,
            sample_rate: 44100.0,
            fade: 0.0,
            fade_target: 0.0,
            fade_inc: 0.0,
            delay_depth: 0.0,
            target_delay_depth: 0.0,
            gain_mod_scale: 1.0,
            noise_mul: 1.0,
            hiss_mul: 1.0,
        }
    }
}

impl Chorus {
    pub fn init(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.line0.init(sample_rate);
        self.line1.init(sample_rate);
        for c in [&mut self.click0, &mut self.click1, &mut self.slow_click0, &mut self.slow_click1] {
            c.init(sample_rate);
        }
        self.leak0.init(sample_rate);
        self.leak1.init(sample_rate);
        self.ring0.init(sample_rate);
        self.ring1.init(sample_rate);
        for p in [&mut self.wet_pink0, &mut self.wet_pink1] {
            p.set_high_shelf(levels::WET_SHELF_HZ, levels::WET_SHELF_DB, sample_rate);
        }
        self.wet_ripple.init(60.0, levels::WET_RIPPLE, sample_rate);
        self.lfo_phase = 0.0;
        self.fade_inc = 1.0 / (FADE_MS * 0.001 * sample_rate);
        if self.mode > 0 {
            self.configure_mode();
            self.delay_depth = self.target_delay_depth;
            self.fade = 1.0;
            self.fade_target = 1.0;
        }
    }

    fn suppress_clicks(&mut self) {
        self.click0.suppress();
        self.click1.suppress();
        self.slow_click0.suppress();
        self.slow_click1.suppress();
        self.ring0.reset();
        self.ring1.reset();
    }

    pub fn set_mode(&mut self, new_mode: u32) {
        let new_mode = new_mode.min(3);
        if new_mode == self.mode && new_mode == self.pending_mode {
            return;
        }
        if self.mode > 0 && new_mode > 0 {
            // Mode-to-mode: fade out, switch at zero, fade back in.
            self.pending_mode = new_mode;
            self.fade_target = 0.0;
            return;
        }
        self.pending_mode = new_mode;
        self.mode = new_mode;
        if self.mode == 0 {
            self.fade_target = 0.0;
            return;
        }
        self.configure_mode();
        self.fade_target = 1.0;
        if self.fade <= 0.0 {
            self.fade = self.fade_inc;
        }
        self.suppress_clicks();
    }

    fn configure_mode(&mut self) {
        let (rate, depth, sine) = match self.mode {
            1 => (CHORUS_I_RATE, CHORUS_I_DEPTH_MS, false),
            2 => (CHORUS_II_RATE, CHORUS_II_DEPTH_MS, false),
            _ => (CHORUS_I_II_RATE, CHORUS_I_II_DEPTH_MS, true),
        };
        self.lfo_inc = rate / self.sample_rate;
        self.target_delay_depth = depth;
        self.gain_mod_scale = depth / CHORUS_I_DEPTH_MS;
        self.use_sine = sine;
    }

    /// Mono in, stereo out. Always runs (even when off) so the delay lines
    /// stay filled and engaging the chorus is click-free.
    #[inline]
    pub fn process(&mut self, input: f32) -> (f32, f32) {
        if self.fade < self.fade_target {
            self.fade = (self.fade + self.fade_inc).min(self.fade_target);
        } else if self.fade > self.fade_target {
            self.fade = (self.fade - self.fade_inc).max(self.fade_target);
        }
        if self.fade <= 0.0 && self.pending_mode != self.mode {
            self.mode = self.pending_mode;
            if self.mode > 0 {
                self.configure_mode();
                self.delay_depth = self.target_delay_depth;
                self.fade_target = 1.0;
                self.suppress_clicks();
            }
        }

        self.delay_depth += (self.target_delay_depth - self.delay_depth) * self.fade_inc;

        self.lfo_phase += self.lfo_inc;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }
        let lfo = if self.use_sine {
            (2.0 * PI * self.lfo_phase).sin()
        } else {
            1.0 - 4.0 * (self.lfo_phase - 0.5).abs()
        };

        let d0_ms = ((CENTER_DELAY_MS + self.delay_depth * lfo) * (1.0 - CLOCK_TRIM)).max(MIN_DELAY_MS);
        let d1_ms = ((CENTER_DELAY_MS - self.delay_depth * lfo) * (1.0 + CLOCK_TRIM)).max(MIN_DELAY_MS);
        let d0 = d0_ms * 0.001 * self.sample_rate;
        let d1 = d1_ms * 0.001 * self.sample_rate;

        let clock0 = (BBD_STAGES / (2.0 * d0_ms * 0.001)).max(MIN_CLOCK_HZ);
        let clock1 = (BBD_STAGES / (2.0 * d1_ms * 0.001)).max(MIN_CLOCK_HZ);

        // Leakage rises dB-linearly with LFO position (~38 dB span).
        let inv_depth = if self.delay_depth > 1e-9 { 1.0 / self.delay_depth } else { 0.0 };
        let n0 = ((d0_ms - CENTER_DELAY_MS) * inv_depth + 1.0) * 0.5;
        let n1 = ((d1_ms - CENTER_DELAY_MS) * inv_depth + 1.0) * 0.5;
        let leak_amt0 = self.delay_depth * (LEAK_MIN_FRAC + (1.0 - LEAK_MIN_FRAC) * n0);
        let leak_amt1 = self.delay_depth * (LEAK_MIN_FRAC + (1.0 - LEAK_MIN_FRAC) * n1);

        let nm = self.noise_mul;
        let hiss = self.hiss_mul;
        let wet_pink0 = self.wet_pink0.process() * levels::WET_BROADBAND * nm * hiss;
        let wet_pink1 = self.wet_pink1.process() * levels::WET_BROADBAND * nm * hiss;
        let leak0 = self.leak0.process() * hiss;
        let leak1 = self.leak1.process() * hiss;

        let gms = self.gain_mod_scale;
        let c0 = self.click0.process(-lfo) * CLICK_GAIN * gms;
        let c1 = self.click1.process(lfo) * CLICK_GAIN * gms;
        let s0 = self.slow_click0.process(lfo) * SLOW_CLICK_GAIN * gms;
        let s1 = self.slow_click1.process(-lfo) * SLOW_CLICK_GAIN * gms;

        let inj0 = (wet_pink0 + leak0 * LEAKAGE_GAIN * leak_amt0 + c0 - s0) * nm;
        let inj1 = (wet_pink1 + leak1 * LEAKAGE_GAIN * leak_amt1 + c1 - s1) * nm;

        let mut wet0 = self.line0.process(input, d0, inj0);
        let mut wet1 = self.line1.process(input, d1, inj1);

        // Clock-rate-dependent charge-transfer efficiency.
        wet0 *= (1.0 + GAIN_TRIM) * (1.0 - CTE_LOSS_COEFF * (1.0 / clock0 - INV_CLOCK_CENTER));
        wet1 *= (1.0 - GAIN_TRIM) * (1.0 - CTE_LOSS_COEFF * (1.0 / clock1 - INV_CLOCK_CENTER));

        wet0 += self.ring0.process(c0) * RING_GAIN * nm;
        wet1 += self.ring1.process(c1) * RING_GAIN * nm;

        let ripple = self.wet_ripple.process() * nm;
        wet0 += ripple;
        wet1 += ripple;

        let dry_mix = 1.0 - self.fade * (1.0 - DRY_GAIN);
        let wet_mix = self.fade * WET_GAIN;
        (dry_mix * input + wet_mix * wet0, dry_mix * input + wet_mix * wet1)
    }
}
