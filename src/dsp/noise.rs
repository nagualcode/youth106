//! Noise sources: the shared NOISE generator and the analog noise floor.
//!
//! Ported from the Ultramaster KR-106 `KR106Noise.h` and
//! `KR106AnalogNoise.h`.

use std::f32::consts::PI;

#[inline]
fn lcg(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(196_314_165).wrapping_add(907_633_515);
    *seed
}

/// Uniform in [-1, 1].
#[inline]
pub fn bipolar(seed: &mut u32) -> f32 {
    2.0 * (lcg(seed) as f32 / u32::MAX as f32) - 1.0
}

const FLICKER_FREQS: [f32; 4] = [30.0, 300.0, 3000.0, 15000.0];
const FLICKER_MIX: f32 = 1.5;

/// NOISE source (one generator for all voices, like the hardware):
/// 2SC945 in reverse breakdown -> 1uF/4.7K HPF (34 Hz) -> BA662 ->
/// 100pF/300K LPF (5305 Hz), with a decade-spaced 1/f tilt bank.
#[derive(Clone)]
pub struct NoiseSource {
    seed: u32,
    lp_state: f32,
    hp_state: f32,
    lp_a: f32,
    hp_a: f32,
    flicker_state: [f32; 4],
    flicker_a: [f32; 4],
}

impl Default for NoiseSource {
    fn default() -> Self {
        Self {
            seed: 22222,
            lp_state: 0.0,
            hp_state: 0.0,
            lp_a: 0.0,
            hp_a: 0.0,
            flicker_state: [0.0; 4],
            flicker_a: [0.0; 4],
        }
    }
}

impl NoiseSource {
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        const LP: f32 = 5305.0;
        const HP: f32 = 34.0;
        let r = LP / sample_rate;
        let fc_lp = LP * (1.0 - 2.2 * r * r);
        self.lp_a = (-2.0 * PI * fc_lp / sample_rate).exp();
        self.hp_a = (-2.0 * PI * HP / sample_rate).exp();
        self.lp_state = 0.0;
        self.hp_state = 0.0;
        for (a, f) in self.flicker_a.iter_mut().zip(FLICKER_FREQS) {
            *a = (-2.0 * PI * f / sample_rate).exp();
        }
        self.flicker_state = [0.0; 4];
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        // 4-sample CLT approximation of gaussian white noise.
        let mut g = 0.0;
        for _ in 0..4 {
            g += bipolar(&mut self.seed);
        }
        let white = g * 0.5;

        let mut flicker = 0.0;
        for (s, a) in self.flicker_state.iter_mut().zip(self.flicker_a) {
            *s = a * *s + (1.0 - a) * white;
            flicker += *s;
        }
        flicker *= 0.25;

        let shaped = white + FLICKER_MIX * flicker;
        self.lp_state = self.lp_a * self.lp_state + (1.0 - self.lp_a) * shaped;
        self.hp_state = self.hp_a * self.hp_state + (1.0 - self.hp_a) * self.lp_state;
        self.lp_state - self.hp_state
    }
}

/// Calibrated analog noise levels (hardware silence recordings, scaled to
/// the DSP saw level).
pub mod levels {
    pub const DRY_BROADBAND: f32 = 1.5e-3;
    pub const DRY_RIPPLE: [f32; 3] = [1.8e-5, 8.9e-6, 6.3e-6];
    pub const WET_BROADBAND: f32 = 1.8e-3;
    pub const WET_SHELF_HZ: f32 = 3000.0;
    pub const WET_SHELF_DB: f32 = 6.0;
    pub const WET_RIPPLE: [f32; 3] = [7.9e-5, 2.2e-5, 9.8e-6];
}

/// Broadband analog floor: pink (Paul Kellet) noise with an optional
/// one-pole high shelf for the HF-tilted chorus path.
#[derive(Clone)]
pub struct FloorNoise {
    seed: u32,
    pink: [f32; 7],
    shelf_state: f32,
    shelf_coeff: f32,
    shelf_gain: f32,
}

impl FloorNoise {
    pub fn new(seed: u32) -> Self {
        Self { seed, pink: [0.0; 7], shelf_state: 0.0, shelf_coeff: 0.0, shelf_gain: 0.0 }
    }

    pub fn set_high_shelf(&mut self, corner_hz: f32, boost_db: f32, sample_rate: f32) {
        let fc = corner_hz.min(sample_rate * 0.45);
        self.shelf_coeff = 1.0 - (-2.0 * PI * fc / sample_rate).exp();
        self.shelf_gain = 10f32.powf(boost_db / 20.0) - 1.0;
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        let white = bipolar(&mut self.seed);
        let p = &mut self.pink;
        p[0] = 0.99886 * p[0] + white * 0.055_517_9;
        p[1] = 0.99332 * p[1] + white * 0.075_075_9;
        p[2] = 0.96900 * p[2] + white * 0.153_852;
        p[3] = 0.86650 * p[3] + white * 0.310_485_6;
        p[4] = 0.55000 * p[4] + white * 0.532_952_2;
        p[5] = -0.7616 * p[5] - white * 0.016_898;
        let pink = p[0] + p[1] + p[2] + p[3] + p[4] + p[5] + p[6] + white * 0.5362;
        p[6] = white * 0.115_926;
        let out = pink * 0.11;
        if self.shelf_gain != 0.0 {
            self.shelf_state += self.shelf_coeff * (out - self.shelf_state);
            out + self.shelf_gain * (out - self.shelf_state)
        } else {
            out
        }
    }
}

/// Mains rail ripple: full-wave rectified (2x mains) plus harmonics.
#[derive(Clone, Default)]
pub struct RailRipple {
    phase: f32,
    inc: f32,
    amps: [f32; 3],
}

impl RailRipple {
    pub fn init(&mut self, mains_hz: f32, amps: [f32; 3], sample_rate: f32) {
        self.inc = 2.0 * mains_hz / sample_rate;
        self.amps = amps;
        self.phase = 0.0;
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let tp = 2.0 * PI * self.phase;
        self.amps[0] * tp.sin() + self.amps[1] * (2.0 * tp).sin() + self.amps[2] * (3.0 * tp).sin()
    }
}
