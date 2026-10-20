//! One Juno-106 voice: DCO -> VCF -> VCA, driven by the firmware tick.
//!
//! Ported from the Ultramaster KR-106 `KR106Voice.h` (J106 path, PolyBLEP
//! oscillator at the oversampled rate).
//!
//! The real D7811G runs one main loop at ~234 Hz. Each pass computes the
//! envelope and the VCF cutoff for every voice, then writes them to a
//! multiplexed DAC one channel at a time: per voice the VCF CV lands
//! ~0.125 ms before the VCA CV, voices are ~0.287 ms apart. Those write
//! offsets, plus the analog RC slews on both CVs, are modeled here.

use super::adsr::{Adsr, ENV_MAX, LOOP_PERIOD_MS, TICK_RATE};
use super::filters::DcBlocker;
use super::firmware::{
    VcfInputs, calc_vcf_bend_amt, calc_vcf_freq, calc_vcf_lfo_signal, dac_to_hz, porta_rate,
    vca_gain,
};
use super::osc::{Dco, NOISE_AMP};
use super::vcf::Vcf;

/// DAC write offsets (fraction of a tick) per physical slot, rebased so
/// slot 0's VCF write lands at the tick rollover.
const PHASE_BASE_MS: f32 = 2.2055;
const VCF_PHASE_MS: [f32; 6] = [2.2055, 2.4929, 2.7803, 3.0677, 3.3552, 3.6426];
const VCA_PHASE_MS: [f32; 6] = [2.3308, 2.6182, 2.9057, 3.1931, 3.4805, 4.0451];
/// Logical voice -> DAC slot. Voices 6-9 (extended polyphony) share the
/// evenly spaced slots 1-4.
const VOICE_SLOT: [usize; 10] = [0, 1, 2, 3, 4, 5, 1, 2, 3, 4];
const WALK_RATE_HZ: [f32; 3] = [0.07, 0.13, 0.31];

/// CV smoothing derived from the hardware RC networks.
const VCA_SLEW_TAU: f32 = 0.000_687; // R106||R105 * C58
const VCF_SLEW_TAU: f32 = 0.000_522; // (VR28+R113)||R110 * C61

/// DCO CV sample-and-hold smoothing. The CV that sets the saw charge
/// current (and so its amplitude) is a per-voice S/H fed by the
/// multiplexed DAC; component values aren't legible in the service notes,
/// so this is an estimate in line with the VCF/VCA CV networks.
const DCO_CV_TAU: f32 = 0.001;

/// Uninitialized glide pitch sentinel (voice never played).
const GLIDE_UNSET: f32 = -100.0;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(196_314_165).wrapping_add(907_633_515);
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

#[derive(Clone)]
pub struct Voice {
    pub osc: Dco,
    pub vcf: Vcf,
    pub adsr: Adsr,

    /// Target pitch in octaves relative to A440.
    pub pitch: f32,
    glide_pitch: f32,
    porta_rate_param: f32,
    porta_step: f32,
    pub midi_note: u8,
    /// DCO RANGE in octaves (switched by resistors, not in the DCO CV).
    pub range_oct: f32,
    hw_res: bool,
    bend_held: f32,
    /// DCO CV: the frequency the CPU last wrote (linear, S/H smoothed).
    dco_cv: f32,
    dco_cv_target: f32,
    dco_cv_coeff: f32,
    dco_cv_latch: bool,

    // --- panel parameters (set by the engine) ---
    pub dco_lfo: f32,
    pub dco_pwm: f32,
    pub dco_sub: f32,
    pub dco_noise: f32,
    pub vcf_res: f32,
    pub bend_dco: f32,
    pub bend_lfo: f32,
    pub raw_bend: f32,
    pub mod_amt: f32,
    /// Octave + transpose + fine tune, in semitones.
    pub transpose_semis: f32,
    pub saw_on: bool,
    pub pulse_on: bool,
    /// -1 = LFO, 0 = manual, 1 = env.
    pub pwm_mode: i32,
    pub vcf_env_positive: bool,
    pub vca_gate: bool,
    velocity: f32,
    pub vcf_cutoff_int: u16,
    pub vcf_env_mod_int: u8,
    pub vcf_key_track_int: u8,
    pub vcf_lfo_depth_int: u8,
    pub vcf_bend_sens_int: u8,
    pub lfo_env_amp: f32,

    // --- per-voice component tolerances ---
    vcf_frq_trim: f32,
    vcf_width_trim: f32,
    vca_gain_scale: f32,
    pw_min_offset: f32,
    pw_max_offset: f32,
    pitch_offset: f32,
    static_offset_unit: f32,
    walk_phase: [f32; 3],
    walk_value: f32,
    voice_index: usize,

    // --- firmware tick / DAC timeline ---
    fw_tick_accum: f32,
    fw_tick_step: f32,
    fw_env_pending: f32,
    fw_env_next: f32,
    vcf_dac_pending: u16,
    vcf_dac_next: u16,
    vcf_dac_updated: bool,
    vca_dac_updated: bool,
    vcf_phase: f32,
    vca_phase: f32,

    // --- analog CV paths ---
    vca_slew: f32,
    vca_slew_coeff: f32,
    vcf_slew: f32,
    vcf_slew_coeff: f32,
    post_osc_dc: DcBlocker,
    post_vcf_dc: DcBlocker,
    last_osc_peak: f32,

    sample_rate: f32,
    inv_nyq: f32,
    oversample: u32,
}

impl Voice {
    pub fn new(voice_index: usize) -> Self {
        let slot = VOICE_SLOT[voice_index.min(9)];
        // Deterministic tolerances seeded by voice index: one specific
        // "unit", identical every session. Magnitudes match the
        // reference's "human tune" defaults.
        let mut rng = Lcg((voice_index as u32).wrapping_mul(2_654_435_761).wrapping_add(0x4675_6E6B));
        let _vcf_freq_offset_j6 = rng.next();
        let vcf_frq_trim = rng.next() * 10.0;
        let vcf_width_trim = 1.0 + rng.next() * (10.0 / 1200.0);
        let vca_gain_scale = 1.0 + rng.next() * 0.024;
        let pw_min_offset = rng.next() * 0.02;
        let pw_max_offset = rng.next() * 0.02;
        let static_offset_unit = rng.next() + rng.next() + rng.next();
        let mut walk_phase = [0.0; 3];
        for p in &mut walk_phase {
            *p = (rng.next() + 1.0) * std::f32::consts::PI;
        }

        // The 8253 counters free-run from power-on, so each DCO sits at an
        // arbitrary (but, in unison, fixed) phase relative to the others.
        let mut osc = Dco::default();
        osc.set_phase((rng.next() + 1.0) * 0.5);

        Self {
            osc,
            vcf: Vcf::default(),
            adsr: Adsr::default(),
            pitch: 0.0,
            glide_pitch: GLIDE_UNSET,
            porta_rate_param: 0.0,
            porta_step: 0.0,
            midi_note: 60,
            range_oct: 0.0,
            hw_res: false,
            bend_held: 0.0,
            dco_cv: 1.0,
            dco_cv_target: 1.0,
            dco_cv_coeff: 1.0,
            dco_cv_latch: true,
            dco_lfo: 0.0,
            dco_pwm: 0.0,
            dco_sub: 1.0,
            dco_noise: 0.0,
            vcf_res: 0.0,
            bend_dco: 0.0,
            bend_lfo: 0.0,
            raw_bend: 0.0,
            mod_amt: 0.0,
            transpose_semis: 0.0,
            saw_on: true,
            pulse_on: true,
            pwm_mode: 0,
            vcf_env_positive: true,
            vca_gate: false,
            velocity: 1.0,
            vcf_cutoff_int: 0x3F80,
            vcf_env_mod_int: 0,
            vcf_key_track_int: 0,
            vcf_lfo_depth_int: 0,
            vcf_bend_sens_int: 0,
            lfo_env_amp: 0.0,
            vcf_frq_trim,
            vcf_width_trim,
            vca_gain_scale,
            pw_min_offset,
            pw_max_offset,
            pitch_offset: 0.0,
            static_offset_unit,
            walk_phase,
            walk_value: 0.0,
            voice_index,
            fw_tick_accum: 0.0,
            fw_tick_step: TICK_RATE / 44100.0,
            fw_env_pending: 0.0,
            fw_env_next: 0.0,
            vcf_dac_pending: 0,
            vcf_dac_next: 0,
            vcf_dac_updated: false,
            vca_dac_updated: false,
            vcf_phase: (VCF_PHASE_MS[slot] - PHASE_BASE_MS) / LOOP_PERIOD_MS,
            vca_phase: (VCA_PHASE_MS[slot] - PHASE_BASE_MS) / LOOP_PERIOD_MS,
            vca_slew: 0.0,
            vca_slew_coeff: 1.0,
            vcf_slew: 0.0,
            vcf_slew_coeff: 1.0,
            post_osc_dc: DcBlocker::default(),
            post_vcf_dc: DcBlocker::default(),
            last_osc_peak: 0.0,
            sample_rate: 44100.0,
            inv_nyq: 1.0 / 22050.0,
            oversample: 2,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32, oversample: u32) {
        self.sample_rate = sample_rate;
        self.oversample = oversample;
        self.vcf.set_sample_rate(sample_rate);
        self.vcf.set_oversample(oversample);
        self.vcf.reset();
        let fs_os = sample_rate * oversample as f32;
        self.osc.init(fs_os);
        // Coupling caps: 10uF into the VCF and 1uF/100K into the VCA
        // (both ~1.59 Hz), at the oversampled rate.
        self.post_osc_dc.set(1.59, fs_os);
        self.post_osc_dc.reset();
        self.post_vcf_dc.set(1.59, fs_os);
        self.post_vcf_dc.reset();
        self.vca_slew_coeff = 1.0 - (-1.0 / (VCA_SLEW_TAU * sample_rate)).exp();
        self.vcf_slew_coeff = 1.0 - (-1.0 / (VCF_SLEW_TAU * sample_rate)).exp();
        self.dco_cv_coeff = 1.0 - (-1.0 / (DCO_CV_TAU * sample_rate)).exp();
        self.inv_nyq = 2.0 / sample_rate;
        self.fw_tick_step = TICK_RATE / sample_rate;
        self.update_porta();
    }

    /// Runtime oversample change: keep the VCF state (held notes keep
    /// sounding), retune everything that depends on the inner rate.
    pub fn set_oversample(&mut self, oversample: u32) {
        self.oversample = oversample;
        self.vcf.set_oversample(oversample);
        let fs_os = self.sample_rate * oversample as f32;
        self.post_osc_dc.set(1.59, fs_os);
        self.post_vcf_dc.set(1.59, fs_os);
    }

    pub fn set_porta_rate(&mut self, slider: f32) {
        self.porta_rate_param = slider;
        self.update_porta();
    }

    fn update_porta(&mut self) {
        // Semitones/sec -> octaves/sample.
        self.porta_step = porta_rate(self.porta_rate_param) / (12.0 * self.sample_rate);
    }

    /// Unison drift (static detune + slow walk). 12 cents static and
    /// +-3 cents walk at drift = 1.
    pub fn set_drift(&mut self, drift: f32, reroll_salt: Option<u32>) {
        if let Some(salt) = reroll_salt {
            let mut rng = Lcg(
                (self.voice_index as u32)
                    .wrapping_mul(2_654_435_761)
                    .wrapping_add(salt)
                    .wrapping_add(0x4472_6974),
            );
            self.static_offset_unit = rng.next() + rng.next() + rng.next();
        }
        self.pitch_offset = self.static_offset_unit * drift * 12.0 / 1200.0 + self.walk_value;
    }

    pub fn update_drift_walk(&mut self, drift: f32, dt: f32) {
        let two_pi = 2.0 * std::f32::consts::PI;
        let mut sum = 0.0;
        for (p, r) in self.walk_phase.iter_mut().zip(WALK_RATE_HZ) {
            *p += two_pi * r * dt;
            if *p > two_pi {
                *p -= two_pi;
            }
            sum += p.sin();
        }
        self.walk_value = (sum / 3.0) * drift * 3.0 / 1200.0;
        self.pitch_offset = self.static_offset_unit * drift * 12.0 / 1200.0 + self.walk_value;
    }

    #[inline]
    fn glide(&mut self, step: f32) {
        let diff = self.pitch - self.glide_pitch;
        if step > 0.0 && diff.abs() > step {
            self.glide_pitch += step.copysign(diff);
        } else {
            self.glide_pitch = self.pitch;
        }
    }

    pub fn busy(&self) -> bool {
        self.adsr.busy()
    }

    fn pitch88(glide_pitch: f32) -> u16 {
        let midi = glide_pitch * 12.0 + 69.0;
        ((midi * 256.0) as i32).clamp(0, 127 << 8) as u16
    }

    fn vcf_inputs(&self, lfo_raw: f32, with_lfo: bool) -> VcfInputs {
        let lfo_signal = if with_lfo {
            let lfo_val = (lfo_raw.abs() * f32::from(0x1FFF_u16)) as u16;
            let depth = (self.lfo_env_amp * 255.0) as u8;
            calc_vcf_lfo_signal(self.vcf_lfo_depth_int, depth, lfo_val)
        } else {
            0
        };
        let bend_val = (self.raw_bend.abs() * 255.0) as u8;
        VcfInputs {
            cutoff: self.vcf_cutoff_int,
            lfo_signal,
            bend_amt: calc_vcf_bend_amt(self.vcf_bend_sens_int, bend_val),
            env_mod: self.vcf_env_mod_int,
            key_track: self.vcf_key_track_int,
            lfo_negative: with_lfo && lfo_raw < 0.0,
            bend_negative: self.raw_bend < 0.0,
            env_positive: self.vcf_env_positive,
            envelope: self.adsr.env_int,
            pitch: Self::pitch88(if self.glide_pitch < -10.0 { -0.75 } else { self.glide_pitch }),
        }
    }

    /// One main-loop pass for this voice: ADSR tick, then the VCF DAC
    /// value computed from the just-updated envelope.
    fn firmware_tick(&mut self, lfo_raw: f32) {
        self.adsr.tick();
        self.fw_env_pending = f32::from(self.adsr.env_int) / f32::from(ENV_MAX);
        self.vcf_dac_pending = calc_vcf_freq(&self.vcf_inputs(lfo_raw, true));
    }

    pub fn trigger(&mut self, level: f32, is_retrigger: bool) {
        self.velocity = level.sqrt();
        if self.glide_pitch < -10.0 {
            self.glide_pitch = self.pitch;
        }
        self.adsr.note_on();
        // note_on already applied the first attack tick: start the CV
        // smoothers there, with the bend applied but no LFO (the onset
        // envelope is always zero at note-on).
        self.fw_env_pending = self.adsr.env_next;
        self.fw_env_next = self.adsr.env_next;
        let dac = calc_vcf_freq(&self.vcf_inputs(0.0, false));
        self.vcf_dac_pending = dac;
        self.vcf_dac_next = dac;
        // Nothing is reset: the VCF keeps self-oscillating between notes
        // and the DCO counters free-run, so a note starts at whatever
        // phase the oscillator is in (hardware behaviour).
        if !is_retrigger {
            self.vca_slew = 0.0;
        }
        // The CPU writes the new divider and DCO CV in the same frame.
        self.dco_cv_latch = true;
    }

    /// New note pitch (octaves re A440). The CPU writes the divider and the
    /// DCO CV for a new note in the same frame.
    pub fn set_pitch(&mut self, pitch: f32) {
        if pitch != self.pitch {
            self.pitch = pitch;
            self.dco_cv_latch = true;
        }
    }

    pub fn release(&mut self) {
        self.adsr.note_off();
    }

    /// Hardware resolution: pitch (portamento, bend) recomputed once per
    /// firmware frame, bend read through the 8-bit bender ADC.
    pub fn set_hw_res(&mut self, on: bool) {
        self.hw_res = on;
        self.bend_held = self.raw_bend;
    }

    pub fn force_finish(&mut self) {
        self.adsr.force_finish();
        self.fw_env_next = 0.0;
        self.fw_env_pending = 0.0;
    }

    /// Render one base-rate sample into `mix` (one slot per oversampled
    /// sub-sample). Idle voices run too, with the VCA closed, so the
    /// filters keep free-running like the hardware.
    #[inline]
    pub fn process(&mut self, lfo: f32, lfo_raw: f32, noise: f32, mix: &mut [f32]) {
        // Portamento: linear glide in octaves (per sample, or per frame
        // in hardware-resolution mode).
        if !self.hw_res {
            self.glide(self.porta_step);
        }

        // Firmware tick + staggered DAC writes.
        self.fw_tick_accum += self.fw_tick_step;
        if self.fw_tick_accum >= 1.0 {
            self.fw_tick_accum -= 1.0;
            if self.hw_res {
                self.glide(self.porta_step / self.fw_tick_step);
                self.bend_held = (self.raw_bend * 255.0).round() / 255.0;
            }
            self.dco_cv_latch = true;
            self.firmware_tick(lfo_raw);
            self.vcf_dac_updated = false;
            self.vca_dac_updated = false;
        }
        if !self.vcf_dac_updated && self.fw_tick_accum >= self.vcf_phase {
            self.vcf_dac_next = self.vcf_dac_pending;
            self.vcf_dac_updated = true;
        }
        if !self.vca_dac_updated && self.fw_tick_accum >= self.vca_phase {
            self.fw_env_next = self.fw_env_pending;
            self.vca_dac_updated = true;
        }
        let env = self.fw_env_next;
        self.adsr.env = env;
        self.adsr.update_gate_env();

        // PWM: LFO mode uses the raw triangle (the onset envelope only
        // gates pitch/VCF depth, ROM $0761-$0788).
        let pw = match self.pwm_mode {
            -1 => self.dco_pwm * (1.0 + lfo_raw) * 0.5,
            1 => self.dco_pwm * env,
            _ => self.dco_pwm,
        };
        let pw_min = 0.5 + self.pw_min_offset;
        let pw_max = 0.95 + self.pw_max_offset;
        let pw = pw_min + pw * (pw_max - pw_min);

        // Pitch: glide + transpose + drift + LFO + bend, all in octaves.
        let bend = if self.hw_res { self.bend_held } else { self.raw_bend };
        let lfo_semis = lfo * (self.dco_lfo + self.mod_amt * self.bend_lfo);
        let octaves = self.glide_pitch
            + self.transpose_semis / 12.0
            + self.pitch_offset
            + lfo_semis / 12.0
            + bend * self.bend_dco;
        let cps = 440.0 * octaves.exp2() / self.sample_rate;
        if cps <= 0.0 || cps >= 0.5 {
            return;
        }

        // Saw amplitude = DCO CV / frequency (C54 charged through
        // R85-87). The CV is written once per frame and smoothed by its
        // S/H, so it only matches the divider in steady state: after pitch
        // jumps, bends or fast vibrato the ramp briefly over/undershoots,
        // and since the pulse is "ramp > PWM CV" its width moves too.
        let f_rel = (octaves - self.range_oct).exp2();
        // Default mode tracks continuously (only the S/H smoothing remains);
        // hardware-resolution mode writes the CV once per frame, and on a
        // note change, like the CPU.
        if self.dco_cv_latch || !self.hw_res {
            self.dco_cv_target = f_rel;
            self.dco_cv_latch = false;
        }
        self.dco_cv += self.dco_cv_coeff * (self.dco_cv_target - self.dco_cv);
        let saw_amp = (self.dco_cv / f_rel).clamp(0.25, 2.0);

        let vcf_cps = dac_to_hz(self.vcf_dac_next, self.vcf_frq_trim, self.vcf_width_trim) * self.inv_nyq;
        self.vcf_slew += self.vcf_slew_coeff * (vcf_cps - self.vcf_slew);
        self.vcf.track_input_env(self.last_osc_peak);
        self.vcf.update_coeffs(self.vcf_slew, self.vcf_res);

        let vca_raw = if self.vca_gate {
            if self.adsr.gate_open() { 1.0 } else { 0.0 }
        } else {
            env
        };
        self.vca_slew += self.vca_slew_coeff * (vca_raw - self.vca_slew);
        let gain = vca_gain(self.vca_slew) * self.velocity * self.vca_gain_scale;

        let cps_os = cps / self.oversample as f32;
        let noise = if self.dco_noise > 0.0 { noise * NOISE_AMP * self.dco_noise } else { 0.0 };
        let mut peak = 0.0_f32;
        for slot in mix.iter_mut() {
            let s = self.osc.process(cps_os, saw_amp, pw, self.saw_on, self.pulse_on, self.dco_sub) + noise;
            let s = self.post_osc_dc.process(s);
            *slot += self.post_vcf_dc.process(self.vcf.process_sample(s)) * gain;
            peak = peak.max(s.abs());
        }
        self.last_osc_peak = peak;
    }
}
