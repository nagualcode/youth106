//! IR3109 4-pole OTA ladder VCF with the BA662 resonance loop.
//!
//! Ported from the Ultramaster KR-106 `KR106VCF_OPTIMIZED.h` (J106
//! resonance curve). Four trapezoidal (TPT) one-pole integrators, each
//! with an OTA tanh nonlinearity solved by one Newton-Raphson step, and
//! global feedback through a circuit-accurate BA662 model:
//!
//! ```text
//!   OscMix --10K--> filter input node <-- OTA output (current)
//!   OscMix --47K--> OTA +IN --1.5K--> GND
//!   FilterOut-100K-> OTA -IN --1.5K--> GND
//! ```
//!
//! Input and feedback share the OTA differential pair, so resonance
//! naturally boosts the passband at low cutoff and the effect fades at
//! high cutoff without any explicit compensation.
//!
//! Control-rate work (`update_coeffs`) runs once per base sample; the
//! audio-rate `process_sample` runs at the oversampled rate.

#[derive(Clone, Copy, Default)]
struct Coeffs {
    k: f32,
    g1: f32,
    g1_2: f32,
    g1_3: f32,
    inv_den: f32,
    k_fb_scale: f32,
    inv_k_fb_scale: f32,
    noise_level: f32,
}

#[derive(Clone)]
pub struct Vcf {
    s: [f32; 4],
    noise_seed: u32,
    input_env: f32,
    env_decay: f32,
    oversample: u32,
    over_scale: f32,
    cache_frq: f32,
    cache_res: f32,
    c: Coeffs,
}

const OTA_SCALE: f32 = 0.35;
/// Direct input gain (OscMix through 10K). With the 8.59 output gain,
/// 0.142 * 8.59 = 1.22: passband near unity at R=0.
const DIRECT_GAIN: f32 = 0.142;
/// Input/feedback attenuation ratio in the BA662 differential pair
/// (1.5/48.5 vs 1.5/101.5), in DSP-normalized units.
const INPUT_RATIO: f32 = 0.065;
const OUTPUT_GAIN: f32 = 8.59;

impl Default for Vcf {
    fn default() -> Self {
        Self {
            s: [0.0; 4],
            noise_seed: 123_456_789,
            input_env: 0.0,
            env_decay: 0.999,
            oversample: 2,
            over_scale: 0.5,
            cache_frq: -1.0,
            cache_res: -1.0,
            c: Coeffs::default(),
        }
    }
}

/// OTA differential-pair tanh (Pade approximant).
#[inline]
fn ota_sat(x: f32) -> f32 {
    if x > 3.0 {
        return 1.0;
    }
    if x < -3.0 {
        return -1.0;
    }
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

#[inline]
fn ota_sat_deriv(x: f32) -> f32 {
    if !(-3.0..=3.0).contains(&x) {
        return 0.0;
    }
    let x2 = x * x;
    let d = 27.0 + 9.0 * x2;
    27.0 * (27.0 - 3.0 * x2) / (d * d)
}

/// Resonance slider -> feedback gain k. Quartic fit to a hardware
/// resonance sweep (noise through the VCF at ~800 Hz, 29 steps), plus a
/// knee above 0.7 and a boost above ~0.9 for self-oscillation onset.
pub fn res_k(res: f32) -> f32 {
    let r = res;
    let r2 = r * r;
    let r3 = r2 * r;
    let r4 = r2 * r2;
    let mut k = 4.6265 * r + 4.3866 * r2 - 8.9869 * r3 + 3.3902 * r4;
    if r > 0.7 {
        let knee = (r - 0.7) / 0.3;
        k += (knee * knee * 1.2).min(0.35);
    }
    let boost = ((r - 0.898) * (0.17 / 0.102)).max(0.0);
    k * (1.0 + boost)
}

/// Nonlinear one-pole OTA-C stage: solves `y = s + g*tanh(x - y)` with one
/// Newton step from the linear TPT estimate.
#[inline]
fn nl_stage(s: &mut f32, x: f32, g: f32, g1: f32) -> f32 {
    let mut y = *s + g1 * (x - *s);
    let sd = (x - y) * OTA_SCALE;
    let t = ota_sat(sd) / OTA_SCALE;
    let f = y - *s - g * t;
    let df = 1.0 + g * ota_sat_deriv(sd);
    y -= f / df;
    *s = 2.0 * y - *s;
    y
}

impl Vcf {
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.env_decay = (-1.0 / (0.022 * sample_rate)).exp();
        self.cache_frq = -1.0;
    }

    pub fn set_oversample(&mut self, os: u32) {
        self.oversample = os.clamp(1, 4);
        self.over_scale = 1.0 / self.oversample as f32;
        self.cache_frq = -1.0;
    }

    pub fn reset(&mut self) {
        self.s = [0.0; 4];
        self.input_env = 0.0;
        self.cache_frq = -1.0;
    }

    #[inline]
    pub fn track_input_env(&mut self, input: f32) {
        self.input_env = input.abs().max(self.input_env * self.env_decay);
    }

    /// Control-rate update. `frq` is cutoff normalized to base Nyquist.
    pub fn update_coeffs(&mut self, frq: f32, res: f32) {
        if frq != self.cache_frq || res != self.cache_res {
            self.cache_frq = frq;
            self.cache_res = res;

            let mut k = res_k(res);
            // At 1x there's no decimator after the VCF: fade resonance near
            // Nyquist so the peak can't fold back.
            if self.oversample == 1 && frq > 0.7 {
                k *= (1.0 - (frq - 0.7) / 0.2).max(0.0);
            }

            let frq_over = (frq * self.over_scale).min(0.85);
            let mut g = (frq_over * std::f32::consts::FRAC_PI_2).tan();

            // Peak-pull compensation, fitted to a 10x10 hardware noise grid.
            let k_fit = k.clamp(0.0, 5.0);
            let mut log_m =
                0.20646 + k_fit * (-0.04140 + k_fit * (0.00602 - k_fit * 0.00127));
            if k < 1.0 {
                log_m *= k.max(0.0);
            }
            if k > 3.7 {
                log_m *= ((4.4 - k) / 0.7).clamp(0.0, 1.0);
            }
            g = (g * log_m.exp()).min(10.0);

            let g1 = g / (1.0 + g);
            let g1_2 = g1 * g1;
            let g1_3 = g1_2 * g1;
            let big_g = g1_2 * g1_2;
            let k_fb_scale = 4.20 * (k - 2.5).clamp(0.3, 1.0);
            self.c = Coeffs {
                k,
                g1,
                g1_2,
                g1_3,
                inv_den: 1.0 / (1.0 + k * big_g),
                k_fb_scale,
                inv_k_fb_scale: 1.0 / k_fb_scale,
                noise_level: self.c.noise_level,
            };
        }

        // Adaptive thermal noise: seeds self-oscillation when quiet, fades
        // when the filter is active so it isn't amplified by the peak.
        let state_energy = self.s.iter().map(|v| v.abs()).sum::<f32>();
        let energy = self.input_env.max(state_energy);
        self.c.noise_level = 1e-2 / (self.oversample as f32 * (1.0 + energy * 1000.0));
    }

    #[inline]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        let c = &self.c;
        self.noise_seed = self.noise_seed.wrapping_mul(196_314_165).wrapping_add(907_633_515);
        let white = f32::from_bits((self.noise_seed >> 9) | 0x4000_0000) - 3.0;
        let input = input + white * c.noise_level;

        // Denormal guard (FTZ-independent).
        self.s[0] += 1e-20;

        // Linear cascade prediction of y4 when u = 0.
        let s_pred = (1.0 - c.g1)
            * (self.s[0] * c.g1_3 + self.s[1] * c.g1_2 + self.s[2] * c.g1 + self.s[3]);

        let ota_diff = input * INPUT_RATIO - s_pred;
        let ota_out = ota_sat(ota_diff * c.k_fb_scale) * c.inv_k_fb_scale;
        let u = (input * DIRECT_GAIN + c.k * ota_out) * c.inv_den;

        // Describing-function pitch compensation for saturated self-osc.
        let state_amp = self.s[3].abs();
        let df_gain = (1.0 / (1.0 + 0.6 * state_amp * state_amp).sqrt()).max(0.65);
        let g1_nl = (c.g1 / df_gain).min(0.98);
        let g_nl = g1_nl / (1.0 - g1_nl);

        let lp1 = nl_stage(&mut self.s[0], u, g_nl, g1_nl);
        let lp2 = nl_stage(&mut self.s[1], lp1, g_nl, g1_nl);
        let lp3 = nl_stage(&mut self.s[2], lp2, g_nl, g1_nl);
        let lp4 = nl_stage(&mut self.s[3], lp3, g_nl, g1_nl);
        lp4 * OUTPUT_GAIN
    }
}
