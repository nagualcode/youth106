//! Youth106 — a Roland Juno-106 emulation for modern DAWs.
//!
//! Plugin layer: host parameters, MIDI handling and the editor. The sound
//! engine lives in [`dsp`].
//!
//! DAW integration choices:
//! - Every panel control is a host parameter with a fixed `id` (never
//!   renumber: hosts store automation by id), a readable name and a
//!   display in real units, so automation lanes read naturally.
//! - Notes come only from the host's MIDI input. Pitch bend, mod wheel
//!   (CC1) and sustain (CC64) are standard MIDI; for VST3 they are routed
//!   through hidden proxy params (`IMidiMapping`), as VST3 requires.
//! - Hold and the sustain pedal are independent: either one holds notes.

use std::sync::Arc;

use truce::prelude::*;
use truce_core::midi::{norm_7bit, norm_pitch_bend};

pub mod dsp;
mod ui;

use dsp::{Engine, HostTime, Param};

// ---------------------------------------------------------------------------
// Switch enums
// ---------------------------------------------------------------------------

#[derive(ParamEnum)]
pub enum ArpModeSw {
    Up,
    #[name = "Up & Down"]
    UpDown,
    Down,
}

#[derive(ParamEnum)]
pub enum ArpRangeSw {
    #[name = "1 Oct"]
    One,
    #[name = "2 Oct"]
    Two,
    #[name = "3 Oct"]
    Three,
}

#[derive(ParamEnum)]
pub enum ArpDivSw {
    #[name = "1/1"]
    D1,
    #[name = "1/2"]
    D2,
    #[name = "1/4"]
    D4,
    #[name = "1/4T"]
    D4T,
    #[name = "1/8"]
    D8,
    #[name = "1/8T"]
    D8T,
    #[name = "1/16"]
    D16,
    #[name = "1/16T"]
    D16T,
    #[name = "1/32"]
    D32,
}

#[derive(ParamEnum)]
pub enum LfoDivSw {
    #[name = "24 bars/4"]
    B24,
    #[name = "16 beats"]
    B16,
    #[name = "8 beats"]
    B8,
    #[name = "1/1"]
    D1,
    #[name = "1/2"]
    D2,
    #[name = "1/4"]
    D4,
    #[name = "1/4T"]
    D4T,
    #[name = "1/8"]
    D8,
    #[name = "1/8T"]
    D8T,
    #[name = "1/16"]
    D16,
    #[name = "1/16T"]
    D16T,
    #[name = "1/32"]
    D32,
    #[name = "1/64"]
    D64,
}

#[derive(ParamEnum)]
pub enum LfoModeSw {
    Auto,
    Manual,
}

#[derive(ParamEnum)]
pub enum RangeSw {
    #[name = "16'"]
    Ft16,
    #[name = "8'"]
    Ft8,
    #[name = "4'"]
    Ft4,
}

#[derive(ParamEnum)]
pub enum PwmModeSw {
    #[name = "LFO"]
    Lfo,
    #[name = "Manual"]
    Manual,
    #[name = "Env"]
    Env,
}

#[derive(ParamEnum)]
pub enum HpfSw {
    #[name = "Boost"]
    Boost,
    #[name = "Flat"]
    Flat,
    #[name = "236 Hz"]
    Hz236,
    #[name = "754 Hz"]
    Hz754,
}

#[derive(ParamEnum)]
pub enum EnvPolSw {
    #[name = "Normal"]
    Normal,
    #[name = "Inverted"]
    Inverted,
}

#[derive(ParamEnum)]
pub enum VcaModeSw {
    #[name = "Env"]
    Env,
    #[name = "Gate"]
    Gate,
}

#[derive(ParamEnum)]
pub enum ChorusSw {
    Off,
    #[name = "I"]
    One,
    #[name = "II"]
    Two,
    /// Juno-6/60 mode; the 106 has only I and II.
    #[name = "I+II (J60)"]
    Both,
}

#[derive(ParamEnum)]
pub enum KeyModeSw {
    #[name = "Mono"]
    Mono,
    #[name = "Poly I"]
    Poly1,
    #[name = "Poly II"]
    Poly2,
}

#[derive(ParamEnum)]
pub enum VoicesSw {
    #[name = "6"]
    Six,
    #[name = "8"]
    Eight,
    #[name = "10"]
    Ten,
}

#[derive(ParamEnum)]
pub enum OversampleSw {
    #[name = "1x"]
    X1,
    #[name = "2x"]
    X2,
    #[name = "4x"]
    X4,
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

#[derive(Params)]
pub struct Youth106Params {
    // --- Arpeggio ---
    #[param(id = 1, name = "Arp On", short_name = "Arp", group = "Arpeggio")]
    pub arp_on: BoolParam,
    #[param(id = 2, name = "Arp Mode", short_name = "Mode", group = "Arpeggio")]
    pub arp_mode: EnumParam<ArpModeSw>,
    #[param(id = 3, name = "Arp Range", short_name = "Range", group = "Arpeggio")]
    pub arp_range: EnumParam<ArpRangeSw>,
    #[param(id = 4, name = "Arp Rate", short_name = "Rate", group = "Arpeggio",
            range = "linear(0, 1)", default = 0.234, format = "fmt_arp_rate")]
    pub arp_rate: FloatParam,
    #[param(id = 5, name = "Arp Sync", short_name = "Sync", group = "Arpeggio")]
    pub arp_sync: BoolParam,
    #[param(id = 6, name = "Arp Division", short_name = "Div", group = "Arpeggio", default = 6)]
    pub arp_div: EnumParam<ArpDivSw>,

    // --- LFO ---
    #[param(id = 10, name = "LFO Rate", short_name = "Rate", group = "LFO",
            range = "linear(0, 1)", default = 0.24, format = "fmt_lfo_rate")]
    pub lfo_rate: FloatParam,
    #[param(id = 11, name = "LFO Delay", short_name = "Delay", group = "LFO",
            range = "linear(0, 1)", default = 0.0, format = "fmt_lfo_delay")]
    pub lfo_delay: FloatParam,
    #[param(id = 12, name = "LFO Trigger", short_name = "Trig", group = "LFO")]
    pub lfo_mode: EnumParam<LfoModeSw>,
    #[param(id = 13, name = "LFO Sync", short_name = "Sync", group = "LFO")]
    pub lfo_sync: BoolParam,
    #[param(id = 14, name = "LFO Division", short_name = "Div", group = "LFO", default = 5)]
    pub lfo_div: EnumParam<LfoDivSw>,

    // --- DCO ---
    #[param(id = 20, name = "DCO Range", short_name = "Range", group = "DCO", default = 1)]
    pub dco_range: EnumParam<RangeSw>,
    #[param(id = 21, name = "DCO LFO", short_name = "LFO", group = "DCO",
            range = "linear(0, 1)", default = 0.0, format = "fmt_dco_lfo")]
    pub dco_lfo: FloatParam,
    #[param(id = 22, name = "DCO PWM", short_name = "PWM", group = "DCO",
            range = "linear(0, 1)", default = 0.5, format = "fmt_pwm")]
    pub dco_pwm: FloatParam,
    #[param(id = 23, name = "PWM Mode", short_name = "PWM Src", group = "DCO", default = 1)]
    pub pwm_mode: EnumParam<PwmModeSw>,
    #[param(id = 24, name = "Pulse", group = "DCO", default = true)]
    pub dco_pulse: BoolParam,
    #[param(id = 25, name = "Saw", group = "DCO", default = true)]
    pub dco_saw: BoolParam,
    #[param(id = 26, name = "Sub Osc", short_name = "Sub", group = "DCO",
            range = "linear(0, 1)", default = 0.0, format = "fmt_ten")]
    pub dco_sub: FloatParam,
    #[param(id = 27, name = "Noise", group = "DCO",
            range = "linear(0, 1)", default = 0.0, format = "fmt_ten")]
    pub dco_noise: FloatParam,

    // --- HPF ---
    #[param(id = 30, name = "HPF", group = "HPF", default = 1)]
    pub hpf: EnumParam<HpfSw>,

    // --- VCF ---
    #[param(id = 40, name = "VCF Freq", short_name = "Freq", group = "VCF",
            range = "linear(0, 1)", default = 1.0, format = "fmt_vcf_freq")]
    pub vcf_freq: FloatParam,
    #[param(id = 41, name = "VCF Resonance", short_name = "Res", group = "VCF",
            range = "linear(0, 1)", default = 0.0, format = "fmt_ten")]
    pub vcf_res: FloatParam,
    #[param(id = 42, name = "VCF Env Polarity", short_name = "Env Pol", group = "VCF")]
    pub vcf_env_pol: EnumParam<EnvPolSw>,
    #[param(id = 43, name = "VCF Env", short_name = "Env", group = "VCF",
            range = "linear(0, 1)", default = 0.0, format = "fmt_ten")]
    pub vcf_env: FloatParam,
    #[param(id = 44, name = "VCF LFO", short_name = "LFO", group = "VCF",
            range = "linear(0, 1)", default = 0.0, format = "fmt_vcf_lfo")]
    pub vcf_lfo: FloatParam,
    #[param(id = 45, name = "VCF Key Follow", short_name = "Kybd", group = "VCF",
            range = "linear(0, 1)", default = 0.0, format = "fmt_ten")]
    pub vcf_kbd: FloatParam,

    // --- VCA ---
    #[param(id = 50, name = "VCA Mode", short_name = "Mode", group = "VCA")]
    pub vca_mode: EnumParam<VcaModeSw>,
    #[param(id = 51, name = "VCA Level", short_name = "Level", group = "VCA",
            range = "linear(0, 1)", default = 0.5, format = "fmt_vca_level")]
    pub vca_level: FloatParam,

    // --- ENV ---
    #[param(id = 60, name = "Attack", short_name = "A", group = "ENV",
            range = "linear(0, 1)", default = 0.0, format = "fmt_attack")]
    pub env_a: FloatParam,
    #[param(id = 61, name = "Decay", short_name = "D", group = "ENV",
            range = "linear(0, 1)", default = 0.25, format = "fmt_dec_rel")]
    pub env_d: FloatParam,
    #[param(id = 62, name = "Sustain", short_name = "S", group = "ENV",
            range = "linear(0, 1)", default = 0.9, format = "fmt_ten")]
    pub env_s: FloatParam,
    #[param(id = 63, name = "Release", short_name = "R", group = "ENV",
            range = "linear(0, 1)", default = 0.25, format = "fmt_dec_rel")]
    pub env_r: FloatParam,

    // --- Chorus ---
    #[param(id = 70, name = "Chorus", group = "Chorus", default = 1)]
    pub chorus: EnumParam<ChorusSw>,

    // --- Controllers ---
    #[param(id = 80, name = "Bend Range", short_name = "Bend", group = "Controller",
            range = "linear(0, 12)", default = 2.0, unit = "st", format = "fmt_bend")]
    pub bend_dco: FloatParam,
    #[param(id = 81, name = "Bend VCF", short_name = "Bnd VCF", group = "Controller",
            range = "linear(0, 1)", default = 0.0, format = "fmt_ten")]
    pub bend_vcf: FloatParam,
    #[param(id = 82, name = "Mod Wheel LFO", short_name = "Mod LFO", group = "Controller",
            range = "linear(0, 1)", default = 0.5, format = "fmt_ten")]
    pub mod_lfo: FloatParam,
    #[param(id = 83, name = "Key Mode", short_name = "Keys", group = "Controller", default = 1)]
    pub key_mode: EnumParam<KeyModeSw>,
    #[param(id = 84, name = "Portamento", short_name = "Porta", group = "Controller",
            range = "linear(0, 1)", default = 0.0, format = "fmt_porta")]
    pub portamento: FloatParam,
    #[param(id = 85, name = "Hold", group = "Controller")]
    pub hold: BoolParam,
    #[param(id = 86, name = "Transpose", short_name = "Trans", group = "Controller",
            range = "discrete(-24, 24)", default = 0, unit = "st")]
    pub transpose: IntParam,
    #[param(id = 87, name = "Fine Tune", short_name = "Tune", group = "Controller",
            range = "linear(-100, 100)", default = 0.0, format = "fmt_cents")]
    pub tune: FloatParam,

    // --- Output / settings ---
    #[param(id = 90, name = "Master Volume", short_name = "Master", group = "Output",
            range = "linear(-60, 12)", default = 0.0, unit = "dB")]
    pub master: FloatParam,
    #[param(id = 91, name = "Voices", group = "Settings")]
    pub voices: EnumParam<VoicesSw>,
    #[param(id = 92, name = "Oversampling", short_name = "OS", group = "Settings", default = 1)]
    pub oversample: EnumParam<OversampleSw>,
    #[param(id = 93, name = "Velocity", short_name = "Vel", group = "Settings")]
    pub velocity: BoolParam,
    #[param(id = 94, name = "Unison Drift", short_name = "Drift", group = "Settings",
            range = "linear(0, 1)", default = 0.0, format = "fmt_ten")]
    pub drift: FloatParam,
    #[param(id = 95, name = "Mono Retrigger", short_name = "Retrig", group = "Settings",
            default = true)]
    pub mono_retrig: BoolParam,
    #[param(id = 96, name = "Analog Noise", short_name = "Noise Flr", group = "Settings",
            range = "linear(0, 1)", default = 1.0, format = "fmt_ten")]
    pub noise_floor: FloatParam,
    /// Hardware resolution: 7-bit panel sliders (PWM 0..105),
    /// per-frame (4.2 ms) portamento/bend and an 8-bit bender, like the
    /// D7811G firmware. Off keeps DAW automation perfectly smooth.
    #[param(id = 97, name = "Hardware Resolution", short_name = "HW Res", group = "Settings")]
    pub hw_res: BoolParam,

    // --- Hidden MIDI proxies (VST3 IMidiMapping; CLAP/AU get raw MIDI) ---
    #[param(id = 120, name = "Pitch Bend", range = "linear(-1, 1)", default = 0.0,
            flags = "hidden | automatable", midi_source = "pitchbend")]
    pub midi_bend: FloatParam,
    #[param(id = 121, name = "Mod Wheel", range = "linear(0, 1)", default = 0.0,
            flags = "hidden | automatable", midi_cc = 1)]
    pub midi_mod: FloatParam,
    #[param(id = 122, name = "Sustain Pedal", range = "linear(0, 1)", default = 0.0,
            flags = "hidden | automatable", midi_cc = 64)]
    pub midi_sustain: FloatParam,

    #[meter]
    pub meter_l: MeterSlot,
    #[meter]
    pub meter_r: MeterSlot,
}

/// Juno sliders are marked 0-10.
fn ten(v: f64) -> String {
    format!("{:.1}", v * 10.0)
}

fn ms(v: f32) -> String {
    if v >= 1000.0 { format!("{:.2} s", v / 1000.0) } else { format!("{v:.0} ms") }
}

impl Youth106Params {
    fn fmt_ten(&self, v: f64) -> String {
        ten(v)
    }
    fn fmt_arp_rate(&self, v: f64) -> String {
        format!("{:.2} Hz", dsp::arp::arp_rate_spm(v as f32) / 60.0)
    }
    fn fmt_lfo_rate(&self, v: f64) -> String {
        format!("{:.2} Hz", dsp::lfo::lfo_rate_hz(v as f32))
    }
    fn fmt_lfo_delay(&self, v: f64) -> String {
        let t = dsp::lfo::lfo_delay_ms(v as f32);
        if t < 1.0 { "Off".into() } else { ms(t) }
    }
    fn fmt_dco_lfo(&self, v: f64) -> String {
        let st = dsp::firmware::dco_lfo_depth(v as f32);
        if st < 0.005 { "Off".into() } else { format!("±{st:.2} st") }
    }
    fn fmt_pwm(&self, v: f64) -> String {
        format!("{:.0}%", 50.0 + 45.0 * v)
    }
    fn fmt_vcf_freq(&self, v: f64) -> String {
        let hz = dsp::firmware::vcf_freq_hz(v as f32);
        if hz >= 1000.0 { format!("{:.2} kHz", hz / 1000.0) } else { format!("{hz:.0} Hz") }
    }
    fn fmt_vcf_lfo(&self, v: f64) -> String {
        let st = dsp::firmware::vcf_lfo_semitones(v as f32);
        if st < 0.05 { "Off".into() } else { format!("±{st:.1} st") }
    }
    fn fmt_vca_level(&self, v: f64) -> String {
        format!("{:+.1} dB", -10.0 + 20.0 * v)
    }
    fn fmt_attack(&self, v: f64) -> String {
        ms(dsp::adsr::attack_ms(v as f32))
    }
    fn fmt_dec_rel(&self, v: f64) -> String {
        ms(dsp::adsr::dec_rel_ms(v as f32))
    }
    fn fmt_cents(&self, v: f64) -> String {
        format!("{v:+.0} ct")
    }
    fn fmt_bend(&self, v: f64) -> String {
        format!("±{v:.1} st")
    }
    fn fmt_porta(&self, v: f64) -> String {
        let t = dsp::firmware::porta_ms_per_oct(v as f32);
        if t <= 0.0 { "Off".into() } else { format!("{} /oct", ms(t)) }
    }
}

use Youth106ParamsParamId as P;

const NUM_ENGINE_PARAMS: usize = 49;

/// Engine parameter order for [`engine_values`].
const ENGINE_PARAMS: [Param; NUM_ENGINE_PARAMS] = [
    Param::ArpOn,
    Param::ArpMode,
    Param::ArpRange,
    Param::ArpRate,
    Param::ArpSync,
    Param::ArpDiv,
    Param::LfoRate,
    Param::LfoDelay,
    Param::LfoMode,
    Param::LfoSync,
    Param::LfoDiv,
    Param::DcoRange,
    Param::DcoLfo,
    Param::DcoPwm,
    Param::PwmMode,
    Param::DcoPulse,
    Param::DcoSaw,
    Param::DcoSub,
    Param::DcoNoise,
    Param::Hpf,
    Param::VcfFreq,
    Param::VcfRes,
    Param::VcfEnvPol,
    Param::VcfEnv,
    Param::VcfLfo,
    Param::VcfKbd,
    Param::VcaMode,
    Param::VcaLevel,
    Param::EnvA,
    Param::EnvD,
    Param::EnvS,
    Param::EnvR,
    Param::Chorus,
    Param::BendDco,
    Param::BendVcf,
    Param::ModLfo,
    // Key mode before Hold/Portamento so a preset load re-seeds once.
    Param::KeyMode,
    Param::Portamento,
    Param::Hold,
    Param::Transpose,
    Param::Tune,
    Param::Master,
    Param::Voices,
    Param::Oversample,
    Param::Velocity,
    Param::Drift,
    Param::MonoRetrig,
    Param::NoiseFloor,
    Param::HwRes,
];

fn b(x: &BoolParam) -> f32 {
    if x.value() { 1.0 } else { 0.0 }
}

fn engine_values(p: &Youth106Params) -> [f32; NUM_ENGINE_PARAMS] {
    [
        b(&p.arp_on),
        p.arp_mode.index() as f32,
        p.arp_range.index() as f32,
        p.arp_rate.value() as f32,
        b(&p.arp_sync),
        p.arp_div.index() as f32,
        p.lfo_rate.value() as f32,
        p.lfo_delay.value() as f32,
        p.lfo_mode.index() as f32,
        b(&p.lfo_sync),
        p.lfo_div.index() as f32,
        p.dco_range.index() as f32,
        p.dco_lfo.value() as f32,
        p.dco_pwm.value() as f32,
        p.pwm_mode.index() as f32,
        b(&p.dco_pulse),
        b(&p.dco_saw),
        p.dco_sub.value() as f32,
        p.dco_noise.value() as f32,
        p.hpf.index() as f32,
        p.vcf_freq.value() as f32,
        p.vcf_res.value() as f32,
        p.vcf_env_pol.index() as f32,
        p.vcf_env.value() as f32,
        p.vcf_lfo.value() as f32,
        p.vcf_kbd.value() as f32,
        p.vca_mode.index() as f32,
        p.vca_level.value() as f32,
        p.env_a.value() as f32,
        p.env_d.value() as f32,
        p.env_s.value() as f32,
        p.env_r.value() as f32,
        p.chorus.index() as f32,
        p.bend_dco.value() as f32,
        p.bend_vcf.value() as f32,
        p.mod_lfo.value() as f32,
        p.key_mode.index() as f32,
        p.portamento.value() as f32,
        b(&p.hold),
        p.transpose.value() as f32,
        p.tune.value() as f32,
        p.master.value() as f32,
        p.voices.index() as f32,
        p.oversample.index() as f32,
        b(&p.velocity),
        p.drift.value() as f32,
        b(&p.mono_retrig),
        p.noise_floor.value() as f32,
        b(&p.hw_res),
    ]
}

/// Steps per full slider travel in hardware-resolution mode: the panel
/// sliders are scanned as 7-bit values (PWM is limited to 105 by R5/R6).
fn hw_steps(p: Param) -> Option<f32> {
    use Param::*;
    match p {
        DcoPwm => Some(105.0),
        LfoRate | LfoDelay | DcoLfo | DcoSub | DcoNoise | VcfFreq | VcfRes | VcfEnv | VcfLfo
        | VcfKbd | VcaLevel | EnvA | EnvD | EnvS | EnvR | BendVcf | ModLfo | Portamento => {
            Some(127.0)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct Youth106 {
    engine: Box<Engine>,
    /// Last values pushed to the engine (NaN = push on next block).
    cache: [f32; NUM_ENGINE_PARAMS],
    peak: [f32; 2],
}

impl Default for Youth106 {
    fn default() -> Self {
        Self { engine: Box::default(), cache: [f32::NAN; NUM_ENGINE_PARAMS], peak: [0.0; 2] }
    }
}

impl Youth106 {
    fn sync_params(&mut self, params: &Youth106Params) {
        let mut vals = engine_values(params);
        if params.hw_res.value() {
            for (v, &p) in vals.iter_mut().zip(&ENGINE_PARAMS) {
                if let Some(n) = hw_steps(p) {
                    *v = (*v * n).round() / n;
                }
            }
        }
        for (i, &v) in vals.iter().enumerate() {
            // `!=` is true for the NaN sentinel, so a reset re-pushes all.
            if v != self.cache[i] {
                self.cache[i] = v;
                self.engine.set(ENGINE_PARAMS[i], v);
            }
        }
    }

    fn handle_event(&mut self, body: &EventBody) {
        match *body {
            EventBody::NoteOn { note, velocity, .. } => self.engine.note_on(note, velocity),
            EventBody::NoteOff { note, .. } => self.engine.note_off(note),
            EventBody::PitchBend { value, .. } => self.engine.pitch_bend(norm_pitch_bend(value)),
            EventBody::ControlChange { cc, value, .. } => match cc {
                1 => self.engine.mod_wheel(norm_7bit(value)),
                64 => self.engine.sustain_pedal(value >= 64),
                120 | 123 => self.engine.all_notes_off(),
                _ => {}
            },
            _ => {}
        }
    }
}

impl PluginLogic for Youth106 {
    type Params = Youth106Params;
    type DspState = Self;

    fn bus_layouts() -> Vec<BusLayout> {
        vec![BusLayout::new().with_output("Main", ChannelConfig::Stereo)]
    }

    fn reset(state: &mut Self, _params: &Youth106Params, config: &AudioConfig) {
        state.engine.reset(config.sample_rate as f32);
        // Re-push every parameter on the next block.
        state.cache = [f32::NAN; NUM_ENGINE_PARAMS];
    }

    fn process(
        state: &mut Self,
        params: &Youth106Params,
        buffer: &mut AudioBuffer,
        events: &EventList,
        context: &mut ProcessContext,
    ) -> ProcessStatus {
        state.sync_params(params);

        let t = context.transport;
        let tempo = if t.tempo > 0.0 { t.tempo } else { 120.0 };
        let beats_per_sample = tempo / 60.0 / context.sample_rate;
        let n = buffer.num_samples();
        let channels = buffer.num_output_channels();

        let mut next = 0;
        let mut peak = [0.0_f32; 2];
        for i in 0..n {
            while let Some(ev) = events.get(next) {
                if ev.sample_offset as usize > i {
                    break;
                }
                state.handle_event(&ev.body);
                next += 1;
            }
            let host = HostTime {
                tempo,
                beat: t.playing.then(|| t.position_beats + i as f64 * beats_per_sample),
            };
            let (l, r) = state.engine.process_sample(host);
            peak[0] = peak[0].max(l.abs());
            peak[1] = peak[1].max(r.abs());
            if channels > 1 {
                buffer.output(0)[i] = l;
                buffer.output(1)[i] = r;
            } else if channels == 1 {
                buffer.output(0)[i] = 0.5 * (l + r);
            }
        }
        while let Some(ev) = events.get(next) {
            state.handle_event(&ev.body);
            next += 1;
        }

        // Peak-hold with ~300 ms release for the UI meters.
        let decay = (-(n as f64) / (0.3 * context.sample_rate)).exp() as f32;
        for (s, p) in state.peak.iter_mut().zip(peak) {
            *s = p.max(*s * decay);
        }
        context.set_meter(P::MeterL, state.peak[0]);
        context.set_meter(P::MeterR, state.peak[1]);

        // The analog noise floor and free-running filters never go
        // silent, exactly like the hardware.
        ProcessStatus::Normal
    }

    fn editor(params: Arc<Youth106Params>) -> Box<dyn Editor> {
        ui::editor(params)
    }
}

truce::plugin! {
    logic: Youth106,
    params: Youth106Params,
}

truce::enable_rt_paranoid!();

#[cfg(test)]
mod tests;
