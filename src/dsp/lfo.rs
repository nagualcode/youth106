//! Juno-106 global LFO (firmware integer triangle + two-stage delay).
//!
//! Ported from the Ultramaster KR-106 `KR106LFO.h` (J106 path). The LFO
//! is a 16-bit accumulator (0..$1FFF) in the D7811G main loop, advanced by
//! a ROM speed coefficient each tick, hard-clamped at both ends; polarity
//! flips once per full accumulator cycle. One audible period is therefore
//! `4 * ceil(8192 / coeff)` ticks — the integer stepping quantizes high
//! rates exactly like the hardware.
//!
//! Optional host sync (a modern addition) keeps the firmware onset
//! envelope but replaces the triangle phase with a tempo-locked one.

use super::adsr::{ENV_MAX, LOOP_PERIOD_MS, attack_inc_from_slider};

/// Byte-exact `0C60_lfoSpeedTbl`.
const LFO_SPEED_TBL: [u16; 128] = [
    0x0005, 0x000f, 0x0019, 0x0028, 0x0037, 0x0046, 0x0050, 0x005a, 0x0064, 0x006e, 0x0078,
    0x0082, 0x008c, 0x0096, 0x00a0, 0x00aa, 0x00b4, 0x00be, 0x00c8, 0x00d2, 0x00dc, 0x00e6,
    0x00f0, 0x00fa, 0x0104, 0x010e, 0x0118, 0x0122, 0x012c, 0x0136, 0x0140, 0x014a, 0x0154,
    0x015e, 0x0168, 0x0172, 0x017c, 0x0186, 0x0190, 0x019a, 0x01a4, 0x01ae, 0x01b8, 0x01c2,
    0x01cc, 0x01d6, 0x01e0, 0x01ea, 0x01f4, 0x01fe, 0x0208, 0x0212, 0x021c, 0x0226, 0x0230,
    0x023a, 0x0244, 0x024e, 0x0258, 0x0262, 0x026c, 0x0276, 0x0280, 0x028a, 0x029a, 0x02aa,
    0x02ba, 0x02ca, 0x02da, 0x02ea, 0x02fa, 0x030a, 0x031a, 0x032a, 0x033a, 0x034a, 0x035a,
    0x036a, 0x037a, 0x038a, 0x039a, 0x03aa, 0x03ba, 0x03ca, 0x03da, 0x03ea, 0x03fa, 0x040a,
    0x041a, 0x042a, 0x043a, 0x044a, 0x045a, 0x046a, 0x047a, 0x048a, 0x04be, 0x04f2, 0x0526,
    0x055a, 0x058e, 0x05c2, 0x05f6, 0x062c, 0x0672, 0x06b8, 0x0708, 0x0758, 0x07a8, 0x07f8,
    0x085c, 0x08c0, 0x0924, 0x0988, 0x09ec, 0x0a50, 0x0ab4, 0x0b18, 0x0b7c, 0x0be0, 0x0c58,
    0x0cd0, 0x0d48, 0x0dde, 0x0e74, 0x0f0a, 0x0fa0, 0x1000,
];

/// `0B30_LfoDelayRampTbl`, indexed by pot >> 4.
const LFO_RAMP_TBL: [u16; 8] = [0xFFFF, 0x0419, 0x020C, 0x015E, 0x0100, 0x0100, 0x0100, 0x0100];

#[inline]
fn slider_byte(slider: f32) -> usize {
    ((slider * 127.0 + 0.5) as i32).clamp(0, 127) as usize
}

/// LFO rate in Hz for a slider position (integer-stepping model).
pub fn lfo_rate_hz(slider: f32) -> f32 {
    let i = slider.clamp(0.0, 1.0) * 127.0;
    let idx = (i as usize).min(126);
    let frac = i - idx as f32;
    let a = f32::from(LFO_SPEED_TBL[idx]);
    let b = f32::from(LFO_SPEED_TBL[idx + 1]);
    let coeff = ((a + frac * (b - a)) + 0.5).max(1.0) as u32;
    let passes_half = 8192_u32.div_ceil(coeff);
    1000.0 / ((4 * passes_half) as f32 * LOOP_PERIOD_MS)
}

/// Delay slider -> approximate total onset time in ms (holdoff + ramp),
/// for display.
pub fn lfo_delay_ms(slider: f32) -> f32 {
    let inc = attack_inc_from_slider(slider);
    let holdoff_ticks =
        if inc >= ENV_MAX { 0.0 } else { f32::from(ENV_MAX) / f32::from(inc) };
    let ramp = LFO_RAMP_TBL[(slider_byte(slider) >> 4).min(7)];
    let ramp_ticks = if ramp == 0xFFFF { 0.0 } else { 65536.0 / f32::from(ramp) };
    (holdoff_ticks + ramp_ticks) * LOOP_PERIOD_MS
}

/// Tempo-sync divisions (beats per LFO cycle).
pub const LFO_DIV_BEATS: [f64; 13] = [
    24.0,
    16.0,
    8.0,
    4.0,
    2.0,
    1.0,
    2.0 / 3.0,
    0.5,
    1.0 / 3.0,
    0.25,
    1.0 / 6.0,
    0.125,
    0.0625,
];

#[derive(Clone)]
pub struct Lfo {
    /// 0 = auto (delay restarts after all keys are up), 1 = manual
    /// (restarts on every gate / mod-wheel trigger).
    pub mode: u32,
    pub trigger: bool,
    gated: bool,
    was_gated: bool,
    armed: bool,

    accum: u16,
    coeff: u16,
    rising: bool,
    polarity: bool,
    /// Held triangle output [-1, 1].
    pub tri: f32,

    holdoff_accum: u16,
    holdoff_inc: u16,
    ramp_accum: u16,
    ramp_inc: u16,
    in_holdoff: bool,
    slider_delay: f32,
    /// Held onset envelope [0, 1].
    pub amp: f32,

    // Host sync.
    pub sync: bool,
    sync_phase: f64,
}

impl Default for Lfo {
    fn default() -> Self {
        Self {
            mode: 0,
            trigger: false,
            gated: false,
            was_gated: false,
            armed: true,
            accum: 0,
            coeff: LFO_SPEED_TBL[30],
            rising: true,
            polarity: false,
            tri: 0.0,
            holdoff_accum: 0,
            holdoff_inc: ENV_MAX,
            ramp_accum: 0,
            ramp_inc: 0xFFFF,
            in_holdoff: false,
            slider_delay: 0.0,
            amp: 0.0,
            sync: false,
            sync_phase: 0.0,
        }
    }
}

impl Lfo {
    pub fn set_rate(&mut self, slider: f32) {
        self.coeff = LFO_SPEED_TBL[slider_byte(slider)];
    }

    pub fn set_delay(&mut self, slider: f32) {
        self.slider_delay = slider;
        self.recalc_delay();
    }

    fn recalc_delay(&mut self) {
        self.holdoff_inc = attack_inc_from_slider(self.slider_delay);
        self.holdoff_accum = 0;
        self.in_holdoff = self.holdoff_inc < ENV_MAX;
        self.ramp_inc = LFO_RAMP_TBL[(slider_byte(self.slider_delay) >> 4).min(7)];
        self.ramp_accum = 0;
        self.amp = 0.0;
    }

    pub fn set_gated(&mut self, gated: bool) {
        self.gated = gated;
    }

    /// Arm/reset logic, mirroring IC29 `$030D-$0323` / `$036B`: in auto
    /// mode the delay restarts only on a key-down after every key was up.
    pub fn update_gate_state(&mut self) {
        let gated = self.gated || self.trigger;
        if self.mode == 1 {
            if gated && !self.was_gated {
                self.recalc_delay();
            }
        } else {
            if !gated {
                self.armed = true;
            }
            if gated && !self.was_gated && self.armed {
                self.armed = false;
                self.recalc_delay();
            }
        }
        self.was_gated = gated;
    }

    /// One firmware tick (`$074E`).
    pub fn tick(&mut self) {
        if self.rising {
            let sum = u32::from(self.accum) + u32::from(self.coeff);
            if sum >= 0x2000 {
                self.accum = 0x1FFF;
                self.rising = false;
            } else {
                self.accum = sum as u16;
            }
        } else if self.coeff > self.accum {
            self.accum = 0;
            self.rising = true;
            self.polarity = !self.polarity;
        } else {
            self.accum -= self.coeff;
        }

        if !self.sync {
            let mag = f32::from(self.accum) / 8191.0;
            self.tri = if self.polarity { -mag } else { mag };
        }

        if self.in_holdoff {
            let sum = u32::from(self.holdoff_accum) + u32::from(self.holdoff_inc);
            if sum >= 0x4000 {
                self.in_holdoff = false;
                self.holdoff_accum = 0x4000;
            } else {
                self.holdoff_accum = sum as u16;
            }
            self.amp = 0.0;
        } else {
            let sum = u32::from(self.ramp_accum) + u32::from(self.ramp_inc);
            if sum >= 0x10000 {
                self.ramp_accum = 0xFFFF;
                self.amp = 1.0;
            } else {
                self.ramp_accum = sum as u16;
                self.amp = f32::from(self.ramp_accum >> 8) / 255.0;
            }
        }
    }

    /// Host-synced triangle. `beat_phase` is `Some(beats)` while the
    /// transport runs (phase locked to the song position); otherwise the
    /// LFO free-runs at the host tempo.
    pub fn advance_sync(&mut self, beats_per_sample: f64, div_beats: f64, beat_pos: Option<f64>) {
        match beat_pos {
            Some(b) => self.sync_phase = (b / div_beats).rem_euclid(1.0),
            None => {
                self.sync_phase += beats_per_sample / div_beats;
                if self.sync_phase >= 1.0 {
                    self.sync_phase -= 1.0;
                }
            }
        }
        // Same shape as the firmware: 0 -> +1 -> 0 -> -1 -> 0.
        let p = self.sync_phase as f32;
        self.tri = if p < 0.25 {
            4.0 * p
        } else if p < 0.75 {
            2.0 - 4.0 * p
        } else {
            4.0 * p - 4.0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_matches_hardware_measurements() {
        // lfrancis unit: coeff 380 -> 2.686 Hz, 1800/1960 -> ~11.7 Hz.
        let hz_for_coeff = |c: u32| 1000.0 / ((4 * 8192_u32.div_ceil(c)) as f32 * LOOP_PERIOD_MS);
        assert!((hz_for_coeff(380) - 2.686).abs() / 2.686 < 0.02);
        assert!((hz_for_coeff(1800) - 11.666).abs() / 11.666 < 0.02);
    }
}
