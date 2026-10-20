//! Juno-106 DCO: saw, pulse and sub from one phase accumulator.
//!
//! Ported from the Ultramaster KR-106 `KR106Oscillators.h` (PolyBLEP
//! oscillator, the reference's default mode). Runs at the oversampled
//! rate. Saw reset, pulse falling edge and sub toggle all coincide at the
//! phase wrap, exactly like the hardware where all three come from the
//! same capacitor ramp.

/// J106 mix levels: MC5534 pre-mixed saw/pulse, separate sub and noise
/// mixing resistors. Calibrated from hardware recordings.
pub const SAW_AMP: f32 = 0.6;
pub const PULSE_AMP: f32 = 0.5;
pub const SUB_AMP: f32 = 0.75;
pub const NOISE_AMP: f32 = 1.2;

/// Gain below which a disabled waveform's body is skipped (-80 dB).
const GAIN_EPS: f32 = 1e-4;

/// 4th-order polyBLEP residual for a discontinuity of height 2, over a
/// 4-sample window (integrated cubic B-spline).
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    let dt2 = dt + dt;
    if t < dt {
        let n = t / dt;
        let n2 = n * n;
        0.25 * n2 * n2 - 0.666_666_7 * n2 * n + 1.333_333_3 * n - 1.0
    } else if t < dt2 {
        let u = 1.0 - (t - dt) / dt;
        let u2 = u * u;
        -0.083_333_33 * u2 * u2
    } else if t > 1.0 - dt {
        let n = (t - 1.0) / dt;
        let n2 = n * n;
        -0.25 * n2 * n2 - 0.666_666_7 * n2 * n + 1.333_333_3 * n + 1.0
    } else if t > 1.0 - dt2 {
        let u = 1.0 + (t - 1.0 + dt) / dt;
        let u2 = u * u;
        0.083_333_33 * u2 * u2
    } else {
        0.0
    }
}

#[derive(Clone)]
pub struct Dco {
    pos: f32,
    sub_state: bool,
    saw_gain: f32,
    pulse_gain: f32,
    sub_gain: f32,
    switch_ramp: f32,
}

impl Default for Dco {
    fn default() -> Self {
        Self {
            pos: 0.0,
            sub_state: false,
            saw_gain: 1.0,
            pulse_gain: 1.0,
            sub_gain: 1.0,
            switch_ramp: 1.0 / 64.0,
        }
    }
}

impl Dco {
    /// `sample_rate` is the rate the oscillator is clocked at (oversampled).
    pub fn init(&mut self, sample_rate: f32) {
        // ~1.45 ms waveform-switch crossfade.
        self.switch_ramp = 1.0 - (-1.0 / (0.00145 * sample_rate)).exp();
    }

    pub fn set_phase(&mut self, phase: f32) {
        self.pos = phase.rem_euclid(1.0);
    }

    /// One sample. `saw_amp` is the ramp height relative to nominal (the
    /// DCO CV / frequency ratio, 1.0 in steady state); `pulse_width` in
    /// [0.5, 0.95] is the duty for a nominal ramp; `sub_level` pre-tapered.
    #[inline]
    pub fn process(
        &mut self,
        cps: f32,
        saw_amp: f32,
        pulse_width: f32,
        saw_on: bool,
        pulse_on: bool,
        sub_level: f32,
    ) -> f32 {
        self.pos += cps;
        if self.pos >= 1.0 {
            self.pos -= 1.0;
            self.sub_state = !self.sub_state;
        }

        let on = |b: bool| if b { 1.0 } else { 0.0 };
        self.saw_gain += (on(saw_on) - self.saw_gain) * self.switch_ramp;
        self.pulse_gain += (on(pulse_on) - self.pulse_gain) * self.switch_ramp;
        // The Juno-106 has no sub switch: the SUB slider alone sets level.
        self.sub_gain += (1.0 - self.sub_gain) * self.switch_ramp;

        // One BLEP at the shared reset edge serves saw, pulse and sub.
        let blep_reset = poly_blep(self.pos, cps);

        let saw = saw_amp * (2.0 * self.pos - 1.0 - blep_reset);
        let mut out = saw * SAW_AMP * self.saw_gain;

        if pulse_on || self.pulse_gain > GAIN_EPS {
            // J106 comparator: the narrow LOW part of the pulse sits at the
            // END of the rising ramp, so the effective width is inverted
            // relative to the J6. The comparator threshold is fixed in volts,
            // so a taller/shorter ramp crosses it earlier/later.
            let eff_pw = ((1.0 - pulse_width) / saw_amp).clamp(0.01, 0.99);
            let mut pulse = if self.pos < eff_pw { -1.0 } else { 1.0 };
            pulse -= blep_reset;
            let mut pw2 = self.pos - eff_pw;
            if pw2 < 0.0 {
                pw2 += 1.0;
            }
            pulse += poly_blep(pw2, cps);
            out += pulse * PULSE_AMP * self.pulse_gain;
        }

        if sub_level > 0.0 || self.sub_gain > GAIN_EPS {
            // CD4013 flip-flop toggled at each reset; |blep| smooths the
            // alternating-direction transitions toward the midpoint.
            let sign = if self.sub_state { -1.0 } else { 1.0 };
            let sub = sign * (1.0 - blep_reset.abs());
            out += sub * SUB_AMP * sub_level * self.sub_gain;
        }

        out
    }
}
