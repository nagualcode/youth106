//! Youth106 DSP engine: a Juno-106 signal chain.
//!
//! The emulation is ported from the Ultramaster KR-106
//! (<https://github.com/kayrockscreenprinting/ultramaster_kr106>, GPL-3.0),
//! keeping only its Juno-106 model. Signal flow:
//!
//! ```text
//!  per voice (6, opt. 8/10):  DCO -> DC block -> IR3109 VCF -> DC block -> VCA
//!  global:  sum (Nx oversampled) -> decimate -> HPF -> + analog floor
//!           -> VCA LEVEL -> BBD chorus (stereo) -> master
//! ```
//!
//! The engine is sample-driven: the plugin calls [`Engine::note_on`] etc.
//! at each event's offset and [`Engine::process_sample`] once per sample.
//! Nothing here allocates after construction.

pub mod adsr;
pub mod arp;
pub mod chorus;
pub mod filters;
pub mod firmware;
pub mod lfo;
pub mod noise;
pub mod osc;
mod tables;
pub mod vcf;
pub mod voice;

use arp::{ArpEvent, Arpeggiator, arp_rate_spm};
use chorus::Chorus;
use filters::{Downsampler2x, Hpf};
use lfo::{LFO_DIV_BEATS, Lfo};
use noise::{FloorNoise, NoiseSource, RailRipple, levels};
use voice::Voice;

pub const MAX_VOICES: usize = 10;

/// Output calibration applied before MASTER: the hardware-level chain puts
/// a single saw voice near -11 dBFS RMS, which makes six-note chords clip
/// a DAW channel. -21 dB lets "Master 0 dB" fit a full open-filter chord
/// under 0 dBFS (single note ~ -20 dBFS RMS).
const OUTPUT_TRIM: f32 = 0.089_125; // -21 dB

/// Engine parameters, in panel order. The plugin layer maps its host
/// parameters onto these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Param {
    ArpOn,
    ArpMode,
    ArpRange,
    ArpRate,
    ArpSync,
    ArpDiv,
    LfoRate,
    LfoDelay,
    LfoMode,
    LfoSync,
    LfoDiv,
    DcoRange,
    DcoLfo,
    DcoPwm,
    PwmMode,
    DcoPulse,
    DcoSaw,
    DcoSub,
    DcoNoise,
    Hpf,
    VcfFreq,
    VcfRes,
    VcfEnvPol,
    VcfEnv,
    VcfLfo,
    VcfKbd,
    VcaMode,
    VcaLevel,
    EnvA,
    EnvD,
    EnvS,
    EnvR,
    Chorus,
    BendDco,
    BendVcf,
    ModLfo,
    KeyMode,
    Portamento,
    Hold,
    Transpose,
    Tune,
    Master,
    Voices,
    Oversample,
    Velocity,
    Drift,
    MonoRetrig,
    NoiseFloor,
    HwRes,
    ChorusHiss,
}

/// Host transport as seen by one sample.
#[derive(Clone, Copy)]
pub struct HostTime {
    pub tempo: f64,
    /// Song position in beats while the transport runs.
    pub beat: Option<f64>,
}

pub struct Engine {
    voices: Vec<Voice>,
    lfo: Lfo,
    hpf: Hpf,
    chorus: Chorus,
    arp: Arpeggiator,
    noise: NoiseSource,
    floor_noise: FloorNoise,
    floor_ripple: RailRipple,
    down1: Downsampler2x,
    down2: Downsampler2x,

    sample_rate: f32,
    oversample: u32,
    bus_lp_state: f32,
    bus_lp_coeff: f32,
    post_vca_lp_state: f32,
    post_vca_lp_coeff: f32,
    dac_smooth_coeff: f32,
    vol_smooth_coeff: f32,
    vca_level: f32,
    vca_level_smooth: f32,
    master: f32,
    master_smooth: f32,
    lfo_tick_accum: f32,
    lfo_tick_step: f32,
    lfo_div: usize,
    noise_floor_mul: f32,
    drift: f32,
    drift_counter: u32,

    // --- performance state ---
    octave: i32,
    tune_semis: f32,
    transpose: i32,
    hold_switch: bool,
    pedal: bool,
    hold: bool,
    /// 0 = Mono (unison), 1 = Poly I, 2 = Poly II.
    key_mode: u32,
    unison_note: Option<u8>,
    unison_stack: Vec<u8>,
    active_voices: usize,
    velocity_on: bool,
    mono_retrigger: bool,
    voice_note: [Option<u8>; MAX_VOICES],
    last_note: [Option<u8>; MAX_VOICES],
    voice_age: [u64; MAX_VOICES],
    age_counter: u64,
    /// IC20 voice priority table: [0] = most recently triggered.
    voice_tbl: [usize; MAX_VOICES],
    keys_down: [bool; 128],
    held_notes: [bool; 128],
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            voices: (0..MAX_VOICES).map(Voice::new).collect(),
            lfo: Lfo::default(),
            hpf: Hpf::default(),
            chorus: Chorus::default(),
            arp: Arpeggiator::default(),
            noise: NoiseSource::default(),
            floor_noise: FloorNoise::new(0x2468_ACE1),
            floor_ripple: RailRipple::default(),
            down1: Downsampler2x::default(),
            down2: Downsampler2x::default(),
            sample_rate: 44100.0,
            oversample: 2,
            bus_lp_state: 0.0,
            bus_lp_coeff: 1.0,
            post_vca_lp_state: 0.0,
            post_vca_lp_coeff: 1.0,
            dac_smooth_coeff: 1.0,
            vol_smooth_coeff: 0.001,
            vca_level: 1.0,
            vca_level_smooth: 1.0,
            master: OUTPUT_TRIM,
            master_smooth: OUTPUT_TRIM,
            lfo_tick_accum: 0.0,
            lfo_tick_step: 0.0,
            lfo_div: 5,
            noise_floor_mul: 1.0,
            drift: 0.0,
            drift_counter: 0,
            octave: 0,
            tune_semis: 0.0,
            transpose: 0,
            hold_switch: false,
            pedal: false,
            hold: false,
            key_mode: 1,
            unison_note: None,
            unison_stack: Vec::with_capacity(128),
            active_voices: 6,
            velocity_on: false,
            mono_retrigger: true,
            voice_note: [None; MAX_VOICES],
            last_note: [None; MAX_VOICES],
            voice_age: [0; MAX_VOICES],
            age_counter: 0,
            voice_tbl: std::array::from_fn(|i| i),
            keys_down: [false; 128],
            held_notes: [false; 128],
        }
    }
}

const DRIFT_UPDATE_SAMPLES: u32 = 64;

impl Engine {
    pub fn reset(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.dac_smooth_coeff = 1.0 - (-1.0 / (0.001 * sample_rate)).exp();
        // ~23 ms volume smoothing (0.001/sample at 44.1 kHz).
        self.vol_smooth_coeff = 1.0 - (-1.0 / (0.0227 * sample_rate)).exp();
        self.lfo_tick_step = adsr::TICK_RATE / sample_rate;
        self.lfo_tick_accum = 0.0;
        // uPC1252H2 VCA output pole.
        self.post_vca_lp_coeff = 1.0 - (-2.0 * std::f32::consts::PI * 100_000.0 / sample_rate).exp();
        self.post_vca_lp_state = 0.0;
        let os = self.oversample;
        for v in &mut self.voices {
            v.set_sample_rate(sample_rate, os);
        }
        self.configure_bus();
        self.voice_note = [None; MAX_VOICES];
        self.unison_note = None;
        self.unison_stack.clear();
        self.reset_voice_tbl();
        self.hpf.init(sample_rate);
        self.chorus.init(sample_rate);
        self.floor_ripple.init(60.0, levels::DRY_RIPPLE, sample_rate);
        self.noise.set_sample_rate(sample_rate);
        self.master_smooth = self.master;
        self.vca_level_smooth = self.vca_level;
    }

    fn configure_bus(&mut self) {
        // 30 kHz 1-pole on the oversampled mix bus before decimation.
        let fs_os = self.sample_rate * self.oversample as f32;
        self.bus_lp_coeff = 1.0 - (-2.0 * std::f32::consts::PI * 30_000.0 / fs_os).exp();
        self.bus_lp_state = 0.0;
        self.down1.clear();
        self.down2.clear();
        // Prime the half-band IIRs so the first note has no startup blip.
        for _ in 0..32 {
            let a = self.down2.process(0.0, 0.0);
            let b = self.down2.process(0.0, 0.0);
            self.down1.process(a, b);
        }
    }

    fn reset_voice_tbl(&mut self) {
        self.voice_tbl = std::array::from_fn(|i| i);
    }

    fn effective_drift(&self) -> f32 {
        // DCOs don't drift on real hardware; the walk is a unison-only
        // musical extra.
        if self.key_mode == 0 { self.drift } else { 0.0 }
    }

    fn update_transpose(&mut self) {
        let semis = self.octave as f32 * 12.0 + self.tune_semis + self.transpose as f32;
        for v in &mut self.voices {
            v.transpose_semis = semis;
        }
    }

    /// Normalized level meter for the UI: true when any voice sounds.
    pub fn any_voice_busy(&self) -> bool {
        self.voices[..self.active_voices].iter().any(Voice::busy)
    }

    pub fn arp_enabled(&self) -> bool {
        self.arp.enabled
    }

    // ------------------------------------------------------------------
    // Parameters
    // ------------------------------------------------------------------

    pub fn set(&mut self, p: Param, v: f32) {
        let on = v > 0.5;
        let idx = v.round().max(0.0) as u32;
        match p {
            Param::ArpOn => self.set_arp_enabled(on),
            Param::ArpMode => self.arp.mode = idx.min(2),
            Param::ArpRange => self.arp.range = idx.min(2),
            Param::ArpRate => self.arp.rate = arp_rate_spm(v),
            Param::ArpSync => self.arp.sync = on,
            Param::ArpDiv => self.arp.division = idx as usize,
            Param::LfoRate => self.lfo.set_rate(v),
            Param::LfoDelay => self.lfo.set_delay(v),
            Param::LfoMode => self.lfo.mode = idx.min(1),
            Param::LfoSync => self.lfo.sync = on,
            Param::LfoDiv => self.lfo_div = (idx as usize).min(LFO_DIV_BEATS.len() - 1),
            Param::DcoRange => {
                self.octave = idx.min(2) as i32 - 1;
                let r = self.octave as f32;
                self.voices.iter_mut().for_each(|x| x.range_oct = r);
                self.update_transpose();
            }
            Param::DcoLfo => {
                let d = firmware::dco_lfo_depth(v);
                self.voices.iter_mut().for_each(|x| x.dco_lfo = d);
            }
            Param::DcoPwm => self.voices.iter_mut().for_each(|x| x.dco_pwm = v),
            Param::PwmMode => {
                let m = idx.min(2) as i32 - 1;
                self.voices.iter_mut().for_each(|x| x.pwm_mode = m);
            }
            Param::DcoPulse => self.voices.iter_mut().for_each(|x| x.pulse_on = on),
            Param::DcoSaw => self.voices.iter_mut().for_each(|x| x.saw_on = on),
            Param::DcoSub => {
                let l = firmware::dco_sub_level(v);
                self.voices.iter_mut().for_each(|x| x.dco_sub = l);
            }
            Param::DcoNoise => {
                let l = firmware::dco_noise_level(v);
                self.voices.iter_mut().for_each(|x| x.dco_noise = l);
            }
            Param::Hpf => self.hpf.set_position(idx as usize),
            Param::VcfFreq => {
                let c = firmware::vcf_cutoff_int(v);
                self.voices.iter_mut().for_each(|x| x.vcf_cutoff_int = c);
            }
            Param::VcfRes => self.voices.iter_mut().for_each(|x| x.vcf_res = v),
            Param::VcfEnvPol => {
                let pos = idx == 0;
                self.voices.iter_mut().for_each(|x| x.vcf_env_positive = pos);
            }
            Param::VcfEnv => {
                let i = (v.clamp(0.0, 1.0) * 254.0) as u8;
                self.voices.iter_mut().for_each(|x| x.vcf_env_mod_int = i);
            }
            Param::VcfLfo => {
                let i = (v.clamp(0.0, 1.0) * 254.0) as u8;
                self.voices.iter_mut().for_each(|x| x.vcf_lfo_depth_int = i);
            }
            Param::VcfKbd => {
                let i = (v.clamp(0.0, 1.0) * 254.0) as u8;
                self.voices.iter_mut().for_each(|x| x.vcf_key_track_int = i);
            }
            Param::VcaMode => {
                let gate = idx == 1;
                for x in &mut self.voices {
                    if gate != x.vca_gate && x.busy() {
                        // Snap so switching mid-note doesn't click.
                        if gate {
                            x.adsr.gate_env = x.adsr.env;
                        } else {
                            x.adsr.env = x.adsr.gate_env;
                        }
                    }
                    x.vca_gate = gate;
                }
            }
            Param::VcaLevel => {
                // uPC1252H2 compander as VCA: +-10 dB, dB-linear (confirmed
                // on hardware: -9.8 / +9.9 dB).
                self.vca_level = 10f32.powf((-10.0 + 20.0 * v) / 20.0);
            }
            Param::EnvA => self.voices.iter_mut().for_each(|x| x.adsr.set_attack(v)),
            Param::EnvD => {
                let i = adsr::dec_rel_index(v);
                self.voices.iter_mut().for_each(|x| x.adsr.set_decay(i));
            }
            Param::EnvS => {
                let s = v.max(0.001);
                self.voices.iter_mut().for_each(|x| x.adsr.set_sustain(s));
            }
            Param::EnvR => {
                let i = adsr::dec_rel_index(v);
                self.voices.iter_mut().for_each(|x| x.adsr.set_release(i));
            }
            Param::Chorus => self.chorus.set_mode(idx.min(3)),
            // Bend range in semitones -> octaves at full deflection.
            Param::BendDco => self.voices.iter_mut().for_each(|x| x.bend_dco = v / 12.0),
            Param::BendVcf => {
                let s = (v.clamp(0.0, 1.0) * 255.0) as u8;
                self.voices.iter_mut().for_each(|x| x.vcf_bend_sens_int = s);
            }
            Param::ModLfo => self.voices.iter_mut().for_each(|x| x.bend_lfo = v),
            Param::KeyMode => self.set_key_mode(idx.min(2)),
            Param::Portamento => self.voices.iter_mut().for_each(|x| x.set_porta_rate(v)),
            Param::Hold => {
                self.hold_switch = on;
                self.update_hold();
            }
            Param::Transpose => {
                self.transpose = v.round() as i32;
                self.update_transpose();
            }
            Param::Tune => {
                // Cents -> semitones.
                self.tune_semis = v / 100.0;
                self.update_transpose();
            }
            Param::Master => {
                self.master = if v <= -60.0 { 0.0 } else { OUTPUT_TRIM * 10f32.powf(v / 20.0) };
            }
            Param::Voices => self.set_active_voices(match idx {
                0 => 6,
                1 => 8,
                _ => 10,
            }),
            Param::Oversample => self.set_oversample(match idx {
                0 => 1,
                1 => 2,
                _ => 4,
            }),
            Param::Velocity => self.velocity_on = on,
            Param::Drift => {
                self.drift = v.clamp(0.0, 1.0);
                let d = self.effective_drift();
                self.voices.iter_mut().for_each(|x| x.set_drift(d, None));
            }
            Param::MonoRetrig => self.mono_retrigger = on,
            Param::HwRes => self.voices.iter_mut().for_each(|x| x.set_hw_res(on)),
            Param::NoiseFloor => {
                self.noise_floor_mul = v;
                self.chorus.noise_mul = v;
            }
            Param::ChorusHiss => self.chorus.hiss_mul = v.clamp(0.0, 1.0),
        }
    }

    fn set_oversample(&mut self, os: u32) {
        if os == self.oversample {
            return;
        }
        self.oversample = os;
        for v in &mut self.voices {
            v.set_oversample(os);
        }
        self.configure_bus();
    }

    fn set_active_voices(&mut self, n: usize) {
        let n = n.clamp(1, MAX_VOICES);
        if n == self.active_voices {
            return;
        }
        // Voices leaving the pool must go idle immediately: they are no
        // longer processed, so a release would never finish.
        for i in n..MAX_VOICES {
            self.voices[i].force_finish();
            self.voice_note[i] = None;
        }
        self.active_voices = n;
        self.reset_voice_tbl();
    }

    fn set_arp_enabled(&mut self, on: bool) {
        let was = self.arp.enabled;
        self.arp.enabled = on;
        if on && !was {
            for n in 0..128u8 {
                if self.keys_down[n as usize] || self.held_notes[n as usize] {
                    self.arp.note_on(n);
                    self.send_to_synth(n, false, 0);
                }
            }
            self.held_notes = [false; 128];
        } else if !on && was {
            if self.hold {
                // Hand the arp's notes back to Hold; the last arp note
                // keeps sounding.
                self.held_notes = [false; 128];
                for &n in &self.arp.held {
                    self.held_notes[n as usize] = true;
                }
                self.arp.last_note = None;
            } else if let Some(n) = self.arp.last_note.take() {
                self.send_to_synth(n, false, 0);
            }
            self.arp.reset();
        }
    }

    fn set_key_mode(&mut self, mode: u32) {
        let prev = self.key_mode;
        if mode == prev {
            return;
        }
        self.key_mode = mode;
        // Poly I <-> Poly II keeps sounding notes; any change involving
        // Mono re-seeds the held notes into the new allocator.
        if prev >= 1 && mode >= 1 {
            return;
        }
        let mut active = [false; 128];
        for (i, a) in active.iter_mut().enumerate() {
            *a = self.keys_down[i] || self.held_notes[i];
        }
        if prev == 0 {
            self.release_unison();
            self.unison_note = None;
            self.unison_stack.clear();
        } else {
            for i in 0..MAX_VOICES {
                if self.voice_note[i].take().is_some() {
                    self.voices[i].release();
                }
            }
        }
        self.held_notes = [false; 128];
        for n in 0..128u8 {
            if active[n as usize] {
                self.send_to_synth(n, true, 127);
                if !self.keys_down[n as usize] {
                    self.held_notes[n as usize] = true;
                }
            }
        }
        let d = self.effective_drift();
        self.voices.iter_mut().for_each(|x| x.set_drift(d, None));
    }

    // ------------------------------------------------------------------
    // MIDI-facing performance input
    // ------------------------------------------------------------------

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        let n = note as usize;
        if n >= 128 || self.keys_down[n] {
            // Edge-trigger dedup (ROM $01E0): a held key can't retrigger.
            return;
        }
        self.keys_down[n] = true;
        if self.arp.enabled {
            self.arp.note_on(note);
            return;
        }
        // A key that was being held by Hold is re-struck: release the
        // held copy first so it can retrigger.
        if self.held_notes[n] {
            self.held_notes[n] = false;
            self.send_to_synth(note, false, 0);
        }
        self.send_to_synth(note, true, velocity);
    }

    pub fn note_off(&mut self, note: u8) {
        let n = note as usize;
        if n >= 128 || !self.keys_down[n] {
            return;
        }
        self.keys_down[n] = false;
        if self.arp.enabled {
            if self.hold {
                self.held_notes[n] = true;
            } else {
                self.arp.note_off(note);
            }
            return;
        }
        if self.hold {
            self.held_notes[n] = true;
            return;
        }
        self.send_to_synth(note, false, 0);
    }

    /// Pitch bend, normalized to [-1, 1].
    pub fn pitch_bend(&mut self, bend: f32) {
        self.voices.iter_mut().for_each(|x| x.raw_bend = bend);
    }

    /// Mod wheel [0, 1]: scales the LFO MOD depth continuously (the
    /// Juno's lever push is on/off; a wheel is more expressive) and
    /// triggers the LFO in manual mode.
    pub fn mod_wheel(&mut self, amount: f32) {
        self.lfo.trigger = amount > 0.0;
        self.voices.iter_mut().for_each(|x| x.mod_amt = amount);
    }

    pub fn sustain_pedal(&mut self, down: bool) {
        self.pedal = down;
        self.update_hold();
    }

    fn update_hold(&mut self) {
        let hold = self.hold_switch || self.pedal;
        if hold == self.hold {
            return;
        }
        self.hold = hold;
        if !hold {
            self.release_held_notes();
        }
    }

    pub fn all_notes_off(&mut self) {
        self.arp.reset();
        self.keys_down = [false; 128];
        self.held_notes = [false; 128];
        for v in &mut self.voices {
            v.release();
        }
        self.voice_note = [None; MAX_VOICES];
        self.reset_voice_tbl();
        self.unison_note = None;
        self.unison_stack.clear();
    }

    fn release_held_notes(&mut self) {
        if self.key_mode == 0 {
            for n in 0..128u8 {
                if self.held_notes[n as usize] {
                    if self.arp.enabled {
                        self.arp.note_off(n);
                    }
                    self.unison_stack.retain(|&x| x != n);
                }
            }
            if let Some(cur) = self.unison_note {
                if self.unison_stack.is_empty() {
                    self.release_unison();
                    self.unison_note = None;
                } else if self.held_notes[cur as usize] {
                    let top = *self.unison_stack.last().unwrap_or(&cur);
                    self.unison_note = Some(top);
                    self.glide_unison(top);
                }
            }
        } else {
            for n in 0..128u8 {
                if self.held_notes[n as usize] {
                    if self.arp.enabled {
                        self.arp.note_off(n);
                    } else {
                        self.send_to_synth(n, false, 0);
                    }
                }
            }
        }
        self.held_notes = [false; 128];
        if self.arp.enabled && !self.arp.held.is_empty() && !self.keys_down.iter().any(|&k| k) {
            if let Some(n) = self.arp.last_note.take() {
                self.send_to_synth(n, false, 0);
            }
            self.arp.held.clear();
        }
        // Safety net: release voices whose key is no longer down.
        for i in 0..MAX_VOICES {
            if let Some(n) = self.voice_note[i] {
                if !self.keys_down[n as usize] {
                    self.voices[i].release();
                    self.voice_note[i] = None;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Voice assignment (IC20 ROM algorithms)
    // ------------------------------------------------------------------

    fn note_pitch(note: u8) -> f32 {
        (f32::from(note) - 69.0) / 12.0
    }

    fn vel_level(&self, velocity: u8) -> f32 {
        if self.velocity_on { f32::from(velocity) / 127.0 } else { 1.0 }
    }

    fn trigger_unison(&mut self, note: u8, velocity: u8) {
        let level = self.vel_level(velocity);
        let n = self.active_voices;
        let any_busy = self.voices[..n].iter().any(Voice::busy);
        for v in &mut self.voices[..n] {
            v.midi_note = note;
            v.set_pitch(Self::note_pitch(note));
            v.trigger(level, any_busy);
        }
    }

    fn glide_unison(&mut self, note: u8) {
        let n = self.active_voices;
        for v in &mut self.voices[..n] {
            v.midi_note = note;
            v.set_pitch(Self::note_pitch(note));
        }
    }

    fn release_unison(&mut self) {
        for v in &mut self.voices {
            v.release();
        }
    }

    /// Poly I: bubble-sort priority table with restart-candidate search
    /// (ROM `$0AAF`). A free voice that last played this note is reused.
    fn find_poly1_voice(&mut self, note: u8) -> Option<usize> {
        let nv = self.active_voices;
        if self.voice_note[self.voice_tbl[nv - 1]].is_some() {
            return None; // all voices busy: note dropped, like hardware
        }
        let mut restart = None;
        let mut first_free = 0;
        for pos in (0..nv).rev() {
            let vi = self.voice_tbl[pos];
            if self.voice_note[vi].is_some() {
                first_free = pos + 1;
                break;
            }
            if self.last_note[vi] == Some(note) {
                restart = Some(pos);
                break;
            }
        }
        if let Some(pos) = restart {
            for p in (1..=pos).rev() {
                self.voice_tbl.swap(p, p - 1);
            }
            Some(self.voice_tbl[0])
        } else {
            Some(self.voice_tbl[first_free])
        }
    }

    fn poly1_note_off(&mut self, note: u8) {
        let nv = self.active_voices;
        let Some(pos) = (0..nv).find(|&p| self.voice_note[self.voice_tbl[p]] == Some(note)) else {
            return;
        };
        let vi = self.voice_tbl[pos];
        self.voices[vi].release();
        self.voice_note[vi] = None;
        for p in pos..nv - 1 {
            self.voice_tbl.swap(p, p + 1);
        }
    }

    fn trigger_voice(&mut self, vi: usize, note: u8, velocity: u8) {
        let level = self.vel_level(velocity);
        let v = &mut self.voices[vi];
        v.midi_note = note;
        v.set_pitch(Self::note_pitch(note));
        let busy = v.busy();
        v.trigger(level, busy);
        self.voice_note[vi] = Some(note);
        self.age_counter += 1;
        self.voice_age[vi] = self.age_counter;
        self.last_note[vi] = Some(note);
    }

    fn send_to_synth(&mut self, note: u8, on: bool, velocity: u8) {
        match self.key_mode {
            0 => {
                if on {
                    self.unison_stack.retain(|&x| x != note);
                    if self.unison_stack.len() < 128 {
                        self.unison_stack.push(note);
                    }
                    let was_playing = self.unison_note.is_some();
                    self.unison_note = Some(note);
                    if was_playing && !self.mono_retrigger {
                        self.glide_unison(note);
                    } else {
                        self.trigger_unison(note, velocity);
                    }
                } else {
                    self.unison_stack.retain(|&x| x != note);
                    if self.unison_note == Some(note) {
                        if let Some(&top) = self.unison_stack.last() {
                            self.unison_note = Some(top);
                            self.glide_unison(top);
                        } else {
                            self.release_unison();
                            self.unison_note = None;
                        }
                    }
                }
            }
            1 => {
                if on {
                    if let Some(vi) = self.find_poly1_voice(note) {
                        self.trigger_voice(vi, note, velocity);
                    }
                } else {
                    self.poly1_note_off(note);
                }
            }
            _ => {
                // Poly II: fixed priority, lowest free voice (ROM $0A76).
                if on {
                    if let Some(vi) = (0..self.active_voices).find(|&i| self.voice_note[i].is_none()) {
                        self.trigger_voice(vi, note, velocity);
                    }
                } else if let Some(i) =
                    (0..self.active_voices).find(|&i| self.voice_note[i] == Some(note))
                {
                    self.voices[i].release();
                    self.voice_note[i] = None;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Audio
    // ------------------------------------------------------------------

    #[inline]
    pub fn process_sample(&mut self, host: HostTime) -> (f32, f32) {
        let nv = self.active_voices;

        // LFO gate follows key-down state (voiceRun), not release tails.
        let gated = self.voice_note[..nv].iter().any(Option::is_some)
            || (self.key_mode == 0 && self.unison_note.is_some());
        self.lfo.set_gated(gated);
        self.lfo.update_gate_state();

        self.lfo_tick_accum += self.lfo_tick_step;
        if self.lfo_tick_accum >= 1.0 {
            self.lfo_tick_accum -= 1.0;
            self.lfo.tick();
        }
        if self.lfo.sync {
            let bps = host.tempo / 60.0 / f64::from(self.sample_rate);
            self.lfo.advance_sync(bps, LFO_DIV_BEATS[self.lfo_div], host.beat);
        }
        let lfo_raw = self.lfo.tri;
        let lfo_amp = self.lfo.amp;
        let lfo = lfo_raw * lfo_amp;
        let noise = self.noise.process();

        for ev in self.arp.process(self.sample_rate, (host.tempo, host.beat)).into_iter().flatten() {
            match ev {
                ArpEvent::Off(n) => self.send_to_synth(n, false, 0),
                ArpEvent::On(n) => self.send_to_synth(n, true, 127),
            }
        }

        self.drift_counter += 1;
        if self.drift_counter >= DRIFT_UPDATE_SAMPLES {
            self.drift_counter = 0;
            let d = self.effective_drift();
            let dt = DRIFT_UPDATE_SAMPLES as f32 / self.sample_rate;
            for v in &mut self.voices {
                v.update_drift_walk(d, dt);
            }
        }

        let os = self.oversample as usize;
        let mut mix = [0.0_f32; 4];
        for v in &mut self.voices[..nv] {
            v.lfo_env_amp = lfo_amp;
            v.process(lfo, lfo_raw, noise, &mut mix[..os]);
        }
        for m in &mut mix[..os] {
            self.bus_lp_state += self.bus_lp_coeff * (*m - self.bus_lp_state);
            *m = self.bus_lp_state;
        }
        let mono = match os {
            4 => {
                let a = self.down2.process(mix[0], mix[1]);
                let b = self.down2.process(mix[2], mix[3]);
                self.down1.process(a, b)
            }
            2 => self.down1.process(mix[0], mix[1]),
            _ => mix[0],
        };

        let mut x = self.hpf.process(mono);
        x += (self.floor_noise.process() * levels::DRY_BROADBAND + self.floor_ripple.process())
            * self.noise_floor_mul;

        // VCA LEVEL, then the VCA output pole, then the chorus.
        self.vca_level_smooth += (self.vca_level - self.vca_level_smooth) * self.dac_smooth_coeff;
        x *= self.vca_level_smooth;
        self.post_vca_lp_state += self.post_vca_lp_coeff * (x - self.post_vca_lp_state);
        let (l, r) = self.chorus.process(self.post_vca_lp_state);

        self.master_smooth += (self.master - self.master_smooth) * self.vol_smooth_coeff;
        (l * self.master_smooth, r * self.master_smooth)
    }
}
