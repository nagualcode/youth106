//! Juno-106 envelope generator.
//!
//! Ported from the Ultramaster KR-106 `KR106ADSR.h` (J106 path only).
//! The Juno-106 envelope is not analog: the D7811G voice CPU computes a
//! 14-bit integer envelope once per main-loop pass (~234 Hz) and writes it
//! to a multiplexed DAC. This module reproduces that integer math:
//!
//! - Attack: linear ramp, `env += atkInc` per tick, clamped at `$3FFF`.
//! - Decay: exponential convergence toward sustain via `CalcDecay`.
//! - Release: exponential toward zero via `CalcDecay`, finishing when the
//!   integer truncates to 0.
//!
//! `CalcDecay` mirrors the ROM's 3-partial-product 16x16 multiply (the
//! `VL*CL` term is never computed), which makes long decays ~6% faster
//! than an exact multiply — matching hardware recordings.

/// Firmware main-loop period. Canonical source for every firmware-rate
/// timing in the plugin (ADSR, LFO, VCF DAC tick, portamento). Average of
/// four hardware LFO-rate measurements once the integer-stepping model is
/// applied (see `lfo.rs`).
pub const LOOP_PERIOD_MS: f32 = 4.2335;
/// Firmware tick rate, ~234.2 Hz.
pub const TICK_RATE: f32 = 1000.0 / LOOP_PERIOD_MS;
/// 14-bit envelope maximum.
pub const ENV_MAX: u16 = 0x3FFF;

/// Gate mode ramp: hardware snaps the VCA fully open/closed in one DAC
/// tick; the BA662 slew smooths it. 32 samples ~ 0.7 ms at 44.1 kHz.
const GATE_SLOPE: f32 = 1.0 / 32.0;

/// Decay/release per-tick multiplier table (0.16 unsigned fixed-point).
/// Piecewise linear, 7 segments, reproducing the ROM table structure
/// (`0D60_envDecRelTbl`). Index 0 = `$1000`, index 127 = `$FFF4`.
const fn gen_dec_rel_table() -> [u16; 128] {
    const COUNTS: [usize; 7] = [4, 1, 10, 28, 22, 58, 4];
    const STEPS: [u16; 7] = [0x2000, 0x1000, 0x0800, 0x0080, 0x000C, 0x0004, 0x0001];
    let mut t = [0u16; 128];
    let mut val: u16 = 0x1000;
    let mut i = 0;
    t[i] = val;
    i += 1;
    let mut seg = 0;
    while seg < 7 {
        let mut n = 0;
        while n < COUNTS[seg] {
            val = val.wrapping_add(STEPS[seg]);
            t[i] = val;
            i += 1;
            n += 1;
        }
        seg += 1;
    }
    t
}

pub static DEC_REL_TABLE: [u16; 128] = gen_dec_rel_table();

/// Attack increment per tick from slider position (0..1).
/// Clean-room piecewise approximation of the ROM attack table
/// (`0B60_envAtkTbl`); 123/128 entries exact, the rest within +-1.
pub fn attack_inc_from_slider(slider: f32) -> u16 {
    let s = slider.clamp(0.0, 1.0);
    if s < 0.003937 {
        return ENV_MAX;
    }
    if s <= 0.5 {
        return (8192.0 / (s * 127.0) + 0.5) as u16;
    }
    if s <= 0.681102 {
        return (305.03 - 352.26 * s + 0.5) as u16;
    }
    if s <= 0.846457 {
        return (194.74 - 190.50 * s + 0.5) as u16;
    }
    if s <= 0.956693 {
        return (86.37 - 62.52 * s + 0.5) as u16;
    }
    ((148.0 - 127.0 * s + 0.5) as i32).max(1) as u16
}

/// ROM-accurate fixed-point multiply (`calcDecay` at `$083D`):
/// `VH*CH + (VH*CL)>>8 + (VL*CH)>>8`, dropping `VL*CL`.
#[inline]
pub fn calc_decay(value: u16, coeff: u16) -> u16 {
    let vh = u32::from(value >> 8);
    let vl = u32::from(value & 0xFF);
    let ch = u32::from(coeff >> 8);
    let cl = u32::from(coeff & 0xFF);
    (vh * ch + ((vh * cl) >> 8) + ((vl * ch) >> 8)) as u16
}

/// Decay/release slider (0..1) to ROM table index (0..127).
#[inline]
pub fn dec_rel_index(slider: f32) -> usize {
    ((slider * 127.0 + 0.5) as i32).clamp(0, 127) as usize
}

/// Attack time in ms for display: 1 ms RC settling + (ticks - 1) periods.
pub fn attack_ms(slider: f32) -> f32 {
    let inc = u32::from(attack_inc_from_slider(slider));
    let ticks = (u32::from(ENV_MAX) + inc - 1) / inc;
    1.0 + (ticks as f32 - 1.0) * (1000.0 / TICK_RATE)
}

/// Decay/release time in ms for display: integer decay from max down to
/// -20 dB, which matches the perceived length on hardware recordings.
pub fn dec_rel_ms(slider: f32) -> f32 {
    let coeff = DEC_REL_TABLE[dec_rel_index(slider)];
    let mut val = ENV_MAX;
    let target = ENV_MAX / 10;
    let mut ticks = 0;
    while val > target && ticks < 50_000 {
        val = calc_decay(val, coeff);
        ticks += 1;
    }
    (ticks as f32 / TICK_RATE * 1000.0).max(1.5)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnvState {
    Attack,
    Decay,
    Release,
    Finished,
}

#[derive(Clone)]
pub struct Adsr {
    pub state: EnvState,
    /// Envelope as seen by the VCA/PWM (normalized, DAC-latched).
    pub env: f32,
    /// Gate-mode envelope ramp.
    pub gate_env: f32,
    /// Current 14-bit envelope (0..=ENV_MAX).
    pub env_int: u16,
    atk_inc: u16,
    dec_mul: u16,
    rel_mul: u16,
    sus_int: u16,
    /// Value right after the immediate first attack tick of a note-on.
    pub env_next: f32,
}

impl Default for Adsr {
    fn default() -> Self {
        Self {
            state: EnvState::Finished,
            env: 0.0,
            gate_env: 0.0,
            env_int: 0,
            atk_inc: ENV_MAX,
            dec_mul: DEC_REL_TABLE[32],
            rel_mul: DEC_REL_TABLE[32],
            sus_int: 0x3F80,
            env_next: 0.0,
        }
    }
}

impl Adsr {
    pub fn set_attack(&mut self, slider: f32) {
        self.atk_inc = attack_inc_from_slider(slider);
    }

    pub fn set_decay(&mut self, index: usize) {
        self.dec_mul = DEC_REL_TABLE[index.min(127)];
    }

    pub fn set_release(&mut self, index: usize) {
        self.rel_mul = DEC_REL_TABLE[index.min(127)];
    }

    /// ROM stores sustain via `store16Bit` (<<7), ceiling `$3F80`. The
    /// envelope peaks at `$3FFF`, so full sustain still takes one decay
    /// tick before snapping — matches hardware.
    pub fn set_sustain(&mut self, s: f32) {
        self.sus_int = (s * f32::from(0x3F80_u16)) as u16;
    }

    pub fn note_on(&mut self) {
        self.state = EnvState::Attack;
        // First attack tick happens in the same loop pass as the note-on.
        // The tick phase is not reset: it stays locked to the voice's VCF
        // tick accumulator, as in the real main loop.
        self.tick();
        self.env_next = f32::from(self.env_int) / f32::from(ENV_MAX);
    }

    pub fn note_off(&mut self) {
        self.state = EnvState::Release;
    }

    pub fn busy(&self) -> bool {
        self.state != EnvState::Finished || self.gate_env > 0.0
    }

    pub fn gate_open(&self) -> bool {
        matches!(self.state, EnvState::Attack | EnvState::Decay)
    }

    /// Per-sample gate envelope update.
    #[inline]
    pub fn update_gate_env(&mut self) {
        if self.gate_open() {
            self.gate_env += GATE_SLOPE;
        } else {
            self.gate_env -= GATE_SLOPE;
        }
        self.gate_env = self.gate_env.clamp(0.0, 1.0);
    }

    /// Force to idle with zeroed envelopes (used when polyphony shrinks).
    pub fn force_finish(&mut self) {
        self.state = EnvState::Finished;
        self.env = 0.0;
        self.gate_env = 0.0;
        self.env_int = 0;
        self.env_next = 0.0;
    }

    /// One D7811G envelope tick (ROM `$0500-$059A`).
    pub fn tick(&mut self) {
        match self.state {
            EnvState::Attack => {
                let sum = u32::from(self.env_int) + u32::from(self.atk_inc);
                if sum >= u32::from(ENV_MAX) {
                    self.env_int = ENV_MAX;
                    self.state = EnvState::Decay;
                } else {
                    self.env_int = sum as u16;
                }
            }
            EnvState::Decay => {
                // Exponential toward sustain; snaps when at/below it
                // (DMOV EA,BC at $0524).
                if self.env_int > self.sus_int {
                    let diff = calc_decay(self.env_int - self.sus_int, self.dec_mul);
                    self.env_int = diff + self.sus_int;
                } else {
                    self.env_int = self.sus_int;
                }
            }
            EnvState::Release => {
                self.env_int = calc_decay(self.env_int, self.rel_mul);
                if self.env_int == 0 {
                    self.state = EnvState::Finished;
                }
            }
            EnvState::Finished => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dec_rel_table_matches_rom_endpoints() {
        assert_eq!(DEC_REL_TABLE[0], 0x1000);
        assert_eq!(DEC_REL_TABLE[127], 0xFFF4);
    }

    #[test]
    fn release_terminates() {
        let mut a = Adsr::default();
        a.set_attack(0.0);
        a.set_release(127);
        a.note_on();
        a.note_off();
        let mut ticks = 0;
        while a.state != EnvState::Finished {
            a.tick();
            ticks += 1;
            assert!(ticks < 100_000);
        }
    }
}
