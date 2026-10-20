//! Small filters shared by the signal chain: half-band decimator, the
//! J106 4-position HPF (with bass boost) and DC blockers.
//!
//! Ported from the Ultramaster KR-106 `KR106Resampler.h`, `KR106_HPF.h`
//! and the HPF struct in `KR106_DSP.h`.

use std::f32::consts::PI;

/// 12-coefficient allpass polyphase half-band (same algorithm and
/// coefficients as Laurent de Soras' HIIR library, WTFPL).
const HALFBAND_COEFS: [f32; 12] = [
    0.036_681_502,
    0.136_547_62,
    0.274_631_76,
    0.423_138_62,
    0.561_098_7,
    0.677_540_05,
    0.769_741_84,
    0.839_889_6,
    0.892_260_8,
    0.931_541_96,
    0.962_094_55,
    0.987_816_4,
];

#[derive(Clone, Default)]
pub struct Downsampler2x {
    x: [f32; 12],
    y: [f32; 12],
}

impl Downsampler2x {
    pub fn clear(&mut self) {
        self.x = [0.0; 12];
        self.y = [0.0; 12];
    }

    #[inline]
    pub fn process(&mut self, a: f32, b: f32) -> f32 {
        let mut s0 = b;
        let mut s1 = a;
        let mut i = 0;
        while i < 12 {
            let t0 = (s0 - self.y[i]) * HALFBAND_COEFS[i] + self.x[i];
            let t1 = (s1 - self.y[i + 1]) * HALFBAND_COEFS[i + 1] + self.x[i + 1];
            self.x[i] = s0;
            self.x[i + 1] = s1;
            self.y[i] = t0;
            self.y[i + 1] = t1;
            s0 = t0;
            s1 = t1;
            i += 2;
        }
        0.5 * (s0 + s1)
    }
}

/// One-pole TPT high-pass used as a coupling-cap DC blocker.
#[derive(Clone, Copy, Default)]
pub struct DcBlocker {
    g: f32,
    s: f32,
}

impl DcBlocker {
    pub fn set(&mut self, corner_hz: f32, sample_rate: f32) {
        let frq = (corner_hz / (sample_rate * 0.5)).clamp(0.0, 0.9);
        self.g = (frq * PI * 0.5).tan();
    }

    pub fn reset(&mut self) {
        self.s = 0.0;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let v = (input - self.s) * self.g / (1.0 + self.g);
        let lp = self.s + v;
        self.s = lp + v;
        input - lp
    }
}

/// HPF position 0: bass boost. Two-stage circuit into the inverting
/// summer; small-signal 2-pole/2-zero model (poles 59/72 Hz, zeros
/// 72/170 Hz, +10.5 dB DC). Verified against a hardware noise sweep
/// (0.55 dB RMS error).
#[derive(Clone, Copy, Default)]
pub struct BassBoost {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl BassBoost {
    pub fn init(&mut self, sample_rate: f32) {
        const R1: f32 = 47e3;
        const C1: f32 = 0.047e-6;
        const CA: f32 = 0.01e-6;
        const RG: f32 = 10e3;
        const RF: f32 = 100e3;
        const CF: f32 = 0.022e-6;
        const R43: f32 = 47e3;
        const R44: f32 = 220e3;
        const R45: f32 = 47e3;

        let tau_1z = R1 * C1;
        let tau_1p = R1 * (C1 + CA);
        let tau_2z = (RG * RF / (RG + RF)) * CF;
        let tau_2p = RF * CF;
        let g2_dc = 1.0 + RF / RG;
        let alpha = R45 / R44;
        let direct = R45 / R43;

        let d0 = 1.0;
        let d1 = tau_1p + tau_2p;
        let d2 = tau_1p * tau_2p;
        let ag = alpha * g2_dc;
        let n0 = direct * d0 + ag;
        let n1 = direct * d1 + ag * (tau_1z + tau_2z);
        let n2 = direct * d2 + ag * (tau_1z * tau_2z);

        let k = 2.0 * sample_rate;
        let k2 = k * k;
        let a0 = d0 + d1 * k + d2 * k2;
        self.b0 = (n0 + n1 * k + n2 * k2) / a0;
        self.b1 = 2.0 * (n0 - n2 * k2) / a0;
        self.b2 = (n0 - n1 * k + n2 * k2) / a0;
        self.a1 = 2.0 * (d0 - d2 * k2) / a0;
        self.a2 = (d0 - d1 * k + d2 * k2) / a0;
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// J106 HPF switch positions (bottom to top): bass boost, flat,
/// 236 Hz, 754 Hz. First-order cuts (CD4051 selects C into 44.9K).
pub fn hpf_freq(position: usize) -> f32 {
    match position {
        0 => -1.0,
        2 => 236.0,
        3 => 754.0,
        _ => 0.0,
    }
}

#[derive(Clone, Copy, Default)]
struct HpfPath {
    freq_hz: f32,
    g: f32,
    hp_s: f32,
    dc_s: f32,
    boost: BassBoost,
}

/// 4-position HPF with a 64-sample crossfade on switch changes.
#[derive(Clone, Default)]
pub struct Hpf {
    sample_rate: f32,
    dc_g: f32,
    cur: HpfPath,
    prev: HpfPath,
    xfade: u32,
}

const HPF_XFADE: u32 = 64;
/// Pre-HPF AC coupling (10uF / 44.9K = 0.35 Hz).
const HPF_DC_HZ: f32 = 0.35;

impl Hpf {
    pub fn init(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let dc = (HPF_DC_HZ / (sample_rate * 0.5)).clamp(0.0, 0.9);
        self.dc_g = (dc * PI * 0.5).tan();
        self.cur.boost.init(sample_rate);
        self.prev.boost.init(sample_rate);
        self.cur.hp_s = 0.0;
        self.cur.dc_s = 0.0;
        self.xfade = 0;
        let f = self.cur.freq_hz;
        self.cur.g = self.coeff(f);
    }

    fn coeff(&self, freq_hz: f32) -> f32 {
        if freq_hz <= 0.0 {
            return 0.0;
        }
        let frq = (freq_hz / (self.sample_rate * 0.5)).clamp(0.001, 0.9);
        (frq * PI * 0.5).tan()
    }

    pub fn set_position(&mut self, position: usize) {
        let f = hpf_freq(position.min(3));
        if f != self.cur.freq_hz {
            self.prev = self.cur;
            self.xfade = HPF_XFADE;
            self.cur.freq_hz = f;
            self.cur.g = self.coeff(f);
        }
    }

    #[inline]
    fn run(path: &mut HpfPath, dc_g: f32, input: f32) -> f32 {
        let x = if path.freq_hz < 0.0 { path.boost.process(input) } else { input };
        // AC-coupling cap, present in all four positions.
        let v = (x - path.dc_s) * dc_g / (1.0 + dc_g);
        let lp = path.dc_s + v;
        path.dc_s = lp + v;
        let x = x - lp;
        if path.freq_hz <= 0.0 {
            return x;
        }
        let v = (x - path.hp_s) * path.g / (1.0 + path.g);
        let lp = path.hp_s + v;
        path.hp_s = lp + v;
        x - lp
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let mut out = Self::run(&mut self.cur, self.dc_g, input);
        if self.xfade > 0 {
            let prev = Self::run(&mut self.prev, self.dc_g, input);
            let t = self.xfade as f32 / HPF_XFADE as f32;
            out = out * (1.0 - t) + prev * t;
            self.xfade -= 1;
        }
        out
    }
}
