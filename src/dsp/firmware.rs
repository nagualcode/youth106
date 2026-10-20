//! Juno-106 voice-CPU (D7811G) control math.
//!
//! Ported from the Ultramaster KR-106 `KR106VcfFreqJ106.h`, `KR106VCA.h`
//! and the J106 depth curves in `KR106Voice.h`. The cutoff calculation is
//! a clean-room reimplementation of the firmware routine at
//! `$04D5-$064E`: all arithmetic is `u16` with explicit carry/borrow
//! tracking, matching the uPD7811's 16-bit EA register pair.

use super::adsr::TICK_RATE;
use super::tables::{V4_HZ, VCA_HW};

/// uPD7811 8x16 multiply: `(coeff * value) >> 8`.
#[inline]
fn mul8x16_hi(coeff: u8, value: u16) -> u16 {
    ((u32::from(coeff) * u32::from(value)) >> 8) as u16
}

/// DADDNC: a carry out clears the underflow flag.
#[inline]
fn vcf_add(ea: u16, bc: u16, underflow: &mut bool) -> u16 {
    let (r, carry) = ea.overflowing_add(bc);
    if carry {
        *underflow = false;
    }
    r
}

/// DSUBNB: a borrow sets the underflow flag.
#[inline]
fn vcf_sub(ea: u16, bc: u16, underflow: &mut bool) -> u16 {
    if bc > ea {
        *underflow = true;
    }
    ea.wrapping_sub(bc)
}

/// Clamp to the 14-bit DAC range; the underflow flag tells a wrapped
/// negative value apart from a genuine overflow.
#[inline]
fn vcf_clamp(ea: u16, underflow: bool) -> u16 {
    if ea > 0x3FFF {
        if underflow { 0 } else { 0x3FFF }
    } else {
        ea
    }
}

/// Inputs to the per-voice VCF cutoff computation.
pub struct VcfInputs {
    /// Slider value `0x0000..=0x3F80`.
    pub cutoff: u16,
    pub lfo_signal: u16,
    pub bend_amt: u16,
    /// Env mod depth 0..254.
    pub env_mod: u8,
    /// Key track depth 0..254.
    pub key_track: u8,
    /// true = subtract LFO.
    pub lfo_negative: bool,
    /// true = subtract bend.
    pub bend_negative: bool,
    /// true = positive envelope polarity.
    pub env_positive: bool,
    /// Per-voice ADSR level `0..=0x3FFF`.
    pub envelope: u16,
    /// 8.8 fixed-point semitone pitch.
    pub pitch: u16,
}

/// One voice's VCF cutoff CV (14-bit DAC value).
pub fn calc_vcf_freq(i: &VcfInputs) -> u16 {
    let mut uf = false;
    let mut ea = i.cutoff;

    ea = if i.lfo_negative {
        vcf_sub(ea, i.lfo_signal, &mut uf)
    } else {
        vcf_add(ea, i.lfo_signal, &mut uf)
    };
    ea = if i.bend_negative {
        vcf_sub(ea, i.bend_amt, &mut uf)
    } else {
        vcf_add(ea, i.bend_amt, &mut uf)
    };

    let scaled_env = mul8x16_hi(i.env_mod, i.envelope);
    ea = if i.env_positive {
        vcf_add(ea, scaled_env, &mut uf)
    } else {
        vcf_sub(ea, scaled_env, &mut uf)
    };

    // Key tracking: pitch * 3/8, centred on middle C (0x3C00 * 0.375).
    const MIDDLE_C_SCALED: u16 = 0x1680;
    let p_scaled = (i.pitch >> 2) + (i.pitch >> 3);
    if p_scaled > MIDDLE_C_SCALED {
        let d = mul8x16_hi(i.key_track, p_scaled - MIDDLE_C_SCALED);
        ea = vcf_add(ea, d, &mut uf);
    } else {
        let d = mul8x16_hi(i.key_track, MIDDLE_C_SCALED - p_scaled);
        ea = vcf_sub(ea, d, &mut uf);
    }

    vcf_clamp(ea, uf)
}

/// LFO -> VCF amount, once per tick.
#[inline]
pub fn calc_vcf_lfo_signal(lfo_to_vcf: u8, depth_scalar: u8, lfo_val: u16) -> u16 {
    let combined = ((u16::from(lfo_to_vcf) * u16::from(depth_scalar)) >> 8) as u8;
    ((u32::from(combined) * u32::from(lfo_val)) >> 9) as u16
}

/// Bend -> VCF amount, once per tick.
#[inline]
pub fn calc_vcf_bend_amt(sens: u8, bend: u8) -> u16 {
    (u16::from(sens) * u16::from(bend)) >> 4
}

/// 14-bit firmware accumulator -> cutoff Hz via the measured V4 card
/// table. `frq_trim` / `width_trim` model the FREQ/WIDTH trimpots and are
/// applied before the DAC's 12-bit truncation.
#[inline]
pub fn dac_to_hz(dac: u16, frq_trim: f32, width_trim: f32) -> f32 {
    let cv = f32::from(dac) * width_trim + frq_trim;
    let internal = ((cv + 0.5) as i32).clamp(0, 16383);
    V4_HZ[(internal >> 2) as usize]
}

/// VCF FREQ slider (0..1) to the firmware's 14-bit cutoff value.
#[inline]
pub fn vcf_cutoff_int(slider: f32) -> u16 {
    (slider.clamp(0.0, 1.0) * f32::from(0x3F80_u16)) as u16
}

/// VCF FREQ slider to Hz (no trims, no modulation) — for display.
pub fn vcf_freq_hz(slider: f32) -> f32 {
    dac_to_hz(vcf_cutoff_int(slider), 0.0, 1.0)
}

/// J106 VCA: TR17 exp converter + BA662, hardware-measured transfer.
#[inline]
pub fn vca_gain(env_norm: f32) -> f32 {
    let idx = env_norm.clamp(0.0, 1.0) * 255.0;
    let i0 = (idx as usize).min(254);
    let frac = idx - i0 as f32;
    VCA_HW[i0] + frac * (VCA_HW[i0 + 1] - VCA_HW[i0])
}

/// DCO LFO depth slider -> peak pitch deviation in semitones (+-4 st max).
///
/// Analog taper of the 50K pot with 10K shunt, `V = x / (1 + 5x - 5x^2)`,
/// then the three-segment firmware depth table shape (`$0A80`).
pub fn dco_lfo_depth(t: f32) -> f32 {
    let t = t / (1.0 + 5.0 * t - 5.0 * t * t);
    let i = t * 127.0;
    if i < 3.0 {
        return 0.0;
    }
    let coeff = if i < 64.0 {
        i - 2.0
    } else if i < 96.0 {
        62.0 + 2.0 * (i - 64.0)
    } else {
        (128.0 + 4.0 * (i - 96.0)).min(255.0)
    };
    (coeff / 255.0) * 4.0
}

/// Inverse of the DCO LFO analog taper. Factory/SysEx patches carry the
/// post-taper ADC byte; this recovers the slider position that produced it.
pub fn dco_lfo_inverse_taper(t_post: f32) -> f32 {
    if t_post <= 0.0 {
        return 0.0;
    }
    if t_post >= 1.0 {
        return 1.0;
    }
    let a = 5.0 * t_post;
    let b = 1.0 - 5.0 * t_post;
    let c = -t_post;
    (-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a)
}

/// VCF LFO depth in semitones: linear, +-3.5 oct at max (ROM `$01B3`).
pub fn vcf_lfo_semitones(t: f32) -> f32 {
    t * 42.0
}

fn lerp_table(table: &[f32; 11], t: f32) -> f32 {
    let idx = t.clamp(0.0, 1.0) * 10.0;
    let i0 = (idx as usize).min(9);
    let frac = idx - i0 as f32;
    table[i0] + frac * (table[i0 + 1] - table[i0])
}

/// Sub oscillator level: DAC -> transistor shunt, measured on hardware.
pub fn dco_sub_level(t: f32) -> f32 {
    const T: [f32; 11] = [
        0.00154, 0.01251, 0.10875, 0.22125, 0.33576, 0.45206, 0.55750, 0.67178, 0.78447,
        0.91132, 1.0,
    ];
    lerp_table(&T, t)
}

/// Noise level: 12-bit DAC -> 2SA1015 -> BA662 Iabc (ngspice-derived
/// soft-knee fit, RMS error < 0.002).
pub fn dco_noise_level(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    const A: f32 = 1.0632;
    const B: f32 = 0.0146;
    const C: f32 = 0.0594;
    let d = t - C;
    A * ((d * d + B * B).sqrt() + d) * 0.5
}

/// Portamento slider -> glide rate in semitones/second.
///
/// Curve shape of the firmware coefficient table (`$0A00`): instant at 0,
/// then three segments (linear -8/idx, linear -2/idx, exponential decay).
/// The coefficient is added to an 8.8 pitch accumulator once per tick.
pub fn porta_rate(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    let i = t * 127.0;
    let coeff = if i <= 25.0 {
        255.0 - 8.0 * (i - 1.0)
    } else if i <= 47.0 {
        63.0 - 2.0 * (i - 25.0)
    } else {
        (18.0 * 0.9625_f32.powf(i - 48.0)).round()
    };
    coeff * TICK_RATE / 256.0
}

/// Portamento time in ms per octave for display (0 = off).
pub fn porta_ms_per_oct(t: f32) -> f32 {
    let r = porta_rate(t);
    if r <= 0.0 { 0.0 } else { 12_000.0 / r }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dac_anchor_248hz() {
        // Service-manual anchor: DAC code 1568 reads 248 Hz.
        assert!((dac_to_hz(1568 << 2, 0.0, 1.0) - 248.0).abs() < 0.01);
    }

    #[test]
    fn inverse_taper_roundtrip() {
        for k in 1..20 {
            let s = k as f32 / 20.0;
            let post = s / (1.0 + 5.0 * s - 5.0 * s * s);
            assert!((dco_lfo_inverse_taper(post) - s).abs() < 1e-4);
        }
    }

    #[test]
    fn vcf_freq_clamps() {
        let full = VcfInputs {
            cutoff: 0x3F80,
            lfo_signal: 0,
            bend_amt: 0,
            env_mod: 254,
            key_track: 254,
            lfo_negative: false,
            bend_negative: false,
            env_positive: true,
            envelope: 0x3FFF,
            pitch: 127 << 8,
        };
        assert_eq!(calc_vcf_freq(&full), 0x3FFF);
        let low = VcfInputs { cutoff: 0, env_positive: false, pitch: 0, ..full };
        assert_eq!(calc_vcf_freq(&low), 0);
    }
}
