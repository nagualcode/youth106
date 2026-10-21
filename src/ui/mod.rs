//! Editor: Juno-106 panel order, Ableton-style flat look.
//!
//! Top row follows the Juno-106 front panel left to right (LFO, DCO, HPF,
//! VCF, VCA, ENV, CHORUS) with vertical faders. The bottom strip holds
//! what the 106 has beside its keyboard (key assign, portamento, hold,
//! bender) plus the arpeggiator and output. Settings swap into the
//! bottom strip from the header.

mod widgets;

use std::sync::Arc;

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use truce::prelude::*;
use truce_core::editor::PluginContextReadF32;
use truce_egui::EguiEditor;

use crate::Youth106Params;
use crate::Youth106ParamsParamId as P;
use widgets::theme::*;
use widgets::{Ctx, fader, knob, meter, section, selector, small, toggle, value_box};

const COL: f32 = 54.0;
const GAP: f32 = 6.0;
const PAD: f32 = 8.0;
const MARGIN: f32 = 10.0;
const HEADER_H: f32 = 34.0;
const TOP_H: f32 = 234.0;
const BOTTOM_H: f32 = 114.0;
const FADER_H: f32 = 136.0;

pub const WIDTH: u32 = 1140;
pub const HEIGHT: u32 = (HEADER_H + MARGIN + TOP_H + GAP + BOTTOM_H + MARGIN) as u32;

pub fn editor(params: Arc<Youth106Params>) -> Box<dyn Editor> {
    EguiEditor::new(params, (WIDTH, HEIGHT), draw)
        .with_visuals(visuals())
        .into_editor()
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.override_text_color = Some(TEXT);
    v
}

fn section_width(cols: usize) -> f32 {
    2.0 * PAD + cols as f32 * COL
}

/// Cell `i` (0-based column) inside a section's content rect.
fn col(content: Rect, i: usize, top: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(content.left() + i as f32 * COL, top), vec2(COL, h))
}

fn inset(r: Rect, dx: f32, dy: f32) -> Rect {
    r.shrink2(vec2(dx, dy))
}

fn draw(ui: &mut egui::Ui, state: &Ctx) {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, CornerRadius::ZERO, BG);

    let settings_id = egui::Id::new("youth106-settings-open");
    let mut settings = ui.data(|d| d.get_temp::<bool>(settings_id).unwrap_or(false));

    header(ui, state, full, &mut settings);
    ui.data_mut(|d| d.insert_temp(settings_id, settings));

    let top_y = full.top() + HEADER_H + MARGIN;
    let mut x = full.left() + MARGIN;
    let mut place = |cols: usize| {
        let r = Rect::from_min_size(pos2(x, top_y), vec2(section_width(cols), TOP_H));
        x += r.width() + GAP;
        r
    };
    let lfo = place(2);
    let dco = place(4);
    let hpf = place(1);
    let vcf = place(5);
    let vca = place(1);
    let env = place(4);
    let chorus = place(1);

    lfo_section(ui, state, lfo);
    dco_section(ui, state, dco);
    hpf_section(ui, state, hpf);
    vcf_section(ui, state, vcf);
    vca_section(ui, state, vca);
    env_section(ui, state, env);
    chorus_section(ui, state, chorus);

    let bottom = Rect::from_min_max(
        pos2(full.left() + MARGIN, top_y + TOP_H + GAP),
        pos2(full.right() - MARGIN, top_y + TOP_H + GAP + BOTTOM_H),
    );
    if settings {
        settings_strip(ui, state, bottom);
    } else {
        performance_strip(ui, state, bottom);
    }

    ui.ctx().request_repaint();
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn header(ui: &mut egui::Ui, state: &Ctx, full: Rect, settings: &mut bool) {
    let p = ui.painter().clone();
    let bar = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
    p.rect_filled(bar, CornerRadius::ZERO, PANEL);
    p.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0_f32, PANEL_LINE));

    // Juno-style colour stripe + wordmark.
    let sx = bar.left() + MARGIN;
    for (k, c) in [JUNO_RED, ACCENT, ON].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(sx + k as f32 * 7.0, bar.top() + 9.0), vec2(4.0, 16.0));
        p.rect_filled(r, CornerRadius::same(1), c);
    }
    let tx = sx + 28.0;
    let g = p.layout_no_wrap("YOUTH".into(), FontId::proportional(17.0), TEXT);
    let w = g.size().x;
    p.galley(pos2(tx, bar.center().y - g.size().y / 2.0), g, TEXT);
    p.text(pos2(tx + w + 3.0, bar.center().y), Align2::LEFT_CENTER, "106", FontId::proportional(17.0), ACCENT);
    p.text(
        pos2(tx + w + 42.0, bar.center().y + 1.0),
        Align2::LEFT_CENTER,
        "POLYPHONIC SYNTHESIZER  ·  JARDIM SONORO",
        small(10.0),
        DIM,
    );

    // Settings button.
    let btn = Rect::from_min_size(pos2(bar.right() - MARGIN - 78.0, bar.top() + 7.0), vec2(78.0, 20.0));
    let resp = ui.interact(btn, ui.id().with("settings-btn"), Sense::click());
    if resp.clicked() {
        *settings = !*settings;
    }
    let fill = if *settings { ON } else if resp.hovered() { CONTROL_HOVER } else { CONTROL };
    p.rect_filled(btn, CornerRadius::same(2), fill);
    p.text(btn.center(), Align2::CENTER_CENTER, "SETTINGS", small(10.0), if *settings { ON_TEXT } else { TEXT });

    // Key mode / voice count readout.
    let info = format!(
        "{}  ·  {} VOICES  ·  OS {}",
        state.format_param(P::KeyMode).to_uppercase(),
        state.format_param(P::Voices),
        state.format_param(P::Oversample),
    );
    p.text(pos2(btn.left() - 14.0, bar.center().y), Align2::RIGHT_CENTER, info, small(10.0), DIM);
}

// ---------------------------------------------------------------------------
// Top row (Juno-106 panel)
// ---------------------------------------------------------------------------

/// Row geometry inside a top-row section.
fn rows(c: Rect) -> (f32, f32, f32) {
    let fader_top = c.top();
    let sw_top = c.top() + FADER_H + 8.0;
    let sw_h = c.bottom() - sw_top;
    (fader_top, sw_top, sw_h)
}

fn lfo_section(ui: &mut egui::Ui, s: &Ctx, r: Rect) {
    let c = section(ui, r, "LFO");
    let (ft, st, sh) = rows(c);
    fader(ui, s, col(c, 0, ft, FADER_H), P::LfoRate, "RATE");
    fader(ui, s, col(c, 1, ft, FADER_H), P::LfoDelay, "DELAY");
    selector(ui, s, inset(col(c, 0, st, sh), 4.0, 0.0), P::LfoMode, &["AUTO", "MAN"], true);
    let c1 = inset(col(c, 1, st, sh), 4.0, 0.0);
    let half = (c1.height() - 4.0) / 2.0;
    toggle(ui, s, Rect::from_min_size(c1.min, vec2(c1.width(), half)), P::LfoSync, "SYNC");
    value_box(ui, s, Rect::from_min_size(pos2(c1.left(), c1.top() + half + 4.0), vec2(c1.width(), half)), P::LfoDiv);
}

fn dco_section(ui: &mut egui::Ui, s: &Ctx, r: Rect) {
    let c = section(ui, r, "DCO");
    let (ft, st, sh) = rows(c);
    fader(ui, s, col(c, 0, ft, FADER_H), P::DcoLfo, "LFO");
    fader(ui, s, col(c, 1, ft, FADER_H), P::DcoPwm, "PWM");
    fader(ui, s, col(c, 2, ft, FADER_H), P::DcoSub, "SUB");
    fader(ui, s, col(c, 3, ft, FADER_H), P::DcoNoise, "NOISE");
    selector(ui, s, inset(col(c, 0, st, sh), 4.0, 0.0), P::DcoRange, &["16'", "8'", "4'"], true);
    selector(ui, s, inset(col(c, 1, st, sh), 4.0, 0.0), P::PwmMode, &["LFO", "MAN", "ENV"], true);
    wave_toggle(ui, s, inset(col(c, 2, st, sh), 4.0, 4.0), P::DcoPulse, Wave::Pulse);
    wave_toggle(ui, s, inset(col(c, 3, st, sh), 4.0, 4.0), P::DcoSaw, Wave::Saw);
}

#[derive(Clone, Copy)]
enum Wave {
    Pulse,
    Saw,
}

/// Waveform on/off button drawn with its waveform glyph, like the
/// 106's pulse/saw buttons.
fn wave_toggle(ui: &mut egui::Ui, s: &Ctx, rect: Rect, id: P, wave: Wave) {
    toggle(ui, s, rect, id, "");
    let on = s.get_param(id) > 0.5;
    let col = if on { ON_TEXT } else { TEXT };
    let g = rect.shrink2(vec2(10.0, 14.0));
    let (l, rr, t, b) = (g.left(), g.right(), g.top(), g.bottom());
    let w = g.width();
    let pts: Vec<Pos2> = match wave {
        Wave::Pulse => vec![
            pos2(l, b),
            pos2(l, t),
            pos2(l + w * 0.35, t),
            pos2(l + w * 0.35, b),
            pos2(l + w * 0.65, b),
            pos2(l + w * 0.65, t),
            pos2(rr, t),
        ],
        Wave::Saw => vec![pos2(l, b), pos2(l + w * 0.5, t), pos2(l + w * 0.5, b), pos2(rr, t), pos2(rr, b)],
    };
    ui.painter().add(Shape::line(pts, Stroke::new(1.6_f32, col)));
}

fn hpf_section(ui: &mut egui::Ui, s: &Ctx, r: Rect) {
    let c = section(ui, r, "HPF");
    let (ft, st, sh) = rows(c);
    fader(ui, s, col(c, 0, ft, FADER_H), P::Hpf, "FREQ");
    let names = ["BOOST", "FLAT", "236", "754"];
    let idx = (s.get_param(P::Hpf) * 3.0).round() as usize;
    let cell = col(c, 0, st, sh);
    for (i, n) in names.iter().enumerate().rev() {
        let y = cell.top() + (3 - i) as f32 * 13.0 + 6.0;
        let c2 = if i == idx { ACCENT } else { DIM };
        ui.painter().text(pos2(cell.center().x, y), Align2::CENTER_CENTER, *n, small(9.0), c2);
    }
}

fn vcf_section(ui: &mut egui::Ui, s: &Ctx, r: Rect) {
    let c = section(ui, r, "VCF");
    let (ft, st, sh) = rows(c);
    fader(ui, s, col(c, 0, ft, FADER_H), P::VcfFreq, "FREQ");
    fader(ui, s, col(c, 1, ft, FADER_H), P::VcfRes, "RES");
    fader(ui, s, col(c, 2, ft, FADER_H), P::VcfEnv, "ENV");
    fader(ui, s, col(c, 3, ft, FADER_H), P::VcfLfo, "LFO");
    fader(ui, s, col(c, 4, ft, FADER_H), P::VcfKbd, "KYBD");
    let pol = Rect::from_min_max(col(c, 2, st, sh).min, col(c, 2, st, sh).max).shrink2(vec2(4.0, 0.0));
    ui.painter().text(pos2(pol.center().x, pol.top()), Align2::CENTER_TOP, "ENV POL", small(9.0), DIM);
    let sel = Rect::from_min_max(pos2(pol.left(), pol.top() + 14.0), pol.max);
    selector(ui, s, sel, P::VcfEnvPol, &["+", "−"], true);
}

fn vca_section(ui: &mut egui::Ui, s: &Ctx, r: Rect) {
    let c = section(ui, r, "VCA");
    let (ft, st, sh) = rows(c);
    fader(ui, s, col(c, 0, ft, FADER_H), P::VcaLevel, "LEVEL");
    selector(ui, s, inset(col(c, 0, st, sh), 4.0, 0.0), P::VcaMode, &["ENV", "GATE"], true);
}

fn env_section(ui: &mut egui::Ui, s: &Ctx, r: Rect) {
    let c = section(ui, r, "ENV");
    let (ft, st, sh) = rows(c);
    fader(ui, s, col(c, 0, ft, FADER_H), P::EnvA, "A");
    fader(ui, s, col(c, 1, ft, FADER_H), P::EnvD, "D");
    fader(ui, s, col(c, 2, ft, FADER_H), P::EnvS, "S");
    fader(ui, s, col(c, 3, ft, FADER_H), P::EnvR, "R");
    let g = Rect::from_min_max(pos2(c.left() + 4.0, st), pos2(c.right() - 4.0, st + sh));
    adsr_view(ui, s, g);
}

/// Envelope shape preview. Segment widths follow sqrt(time) so short and
/// long settings both stay readable.
fn adsr_view(ui: &egui::Ui, s: &Ctx, rect: Rect) {
    use crate::dsp::adsr::{attack_ms, dec_rel_ms};
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(2), TRACK);
    let a = attack_ms(s.get_param(P::EnvA));
    let d = dec_rel_ms(s.get_param(P::EnvD));
    let sus = s.get_param(P::EnvS);
    let rel = dec_rel_ms(s.get_param(P::EnvR));
    let full = 12_000_f32.sqrt();
    let inner = rect.shrink(5.0);
    let wa = a.sqrt() / full;
    let wd = d.sqrt() / full;
    let wr = rel.sqrt() / full;
    let ws = 0.18;
    let total = (wa + wd + ws + wr).max(1e-3);
    let sx = inner.width() / total.max(1.0);
    let x = |t: f32| inner.left() + t * sx;
    let y = |v: f32| inner.bottom() - v * inner.height();

    let mut pts = vec![pos2(x(0.0), y(0.0)), pos2(x(wa), y(1.0))];
    let n = 24;
    // -20 dB at the nominal time: exp(-ln 10 * t).
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let v = sus + (1.0 - sus) * (-2.3026 * t).exp();
        pts.push(pos2(x(wa + wd * t), y(v)));
    }
    let s_end = wa + wd + ws;
    pts.push(pos2(x(s_end), y(sus)));
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let v = sus * (-2.3026 * t * 1.6).exp();
        pts.push(pos2(x(s_end + wr * t), y(v)));
    }
    let mut fill = pts.clone();
    fill.push(pos2(x(s_end + wr), y(0.0)));
    p.add(Shape::convex_polygon(fill, ACCENT.gamma_multiply(0.12), Stroke::NONE));
    p.add(Shape::line(pts, Stroke::new(1.5_f32, ACCENT)));
}

fn chorus_section(ui: &mut egui::Ui, s: &Ctx, r: Rect) {
    let c = section(ui, r, "CHORUS");
    // The 106 has two buttons (I / II). Both together selects the
    // Juno-60 I+II mode, an extra the 106 itself doesn't have.
    let mode = (s.get_param(P::Chorus) * 3.0).round() as u32;
    let cell = col(c, 0, c.top() + 10.0, 120.0).shrink2(vec2(4.0, 0.0));
    let bh = 48.0;
    for (k, (bit, label)) in [(1u32, "I"), (2u32, "II")].into_iter().enumerate() {
        let b = Rect::from_min_size(pos2(cell.left(), cell.top() + k as f32 * (bh + 10.0)), vec2(cell.width(), bh));
        let resp = ui.interact(b, ui.id().with(("chorus", bit)), Sense::click());
        let on = mode & bit != 0;
        if resp.clicked() {
            let new = mode ^ bit;
            s.automate(P::Chorus, f64::from(new) / 3.0);
        }
        let fill = if on { ON } else if resp.hovered() { CONTROL_HOVER } else { CONTROL };
        ui.painter().rect_filled(b, CornerRadius::same(2), fill);
        ui.painter().text(b.center(), Align2::CENTER_CENTER, label, FontId::proportional(15.0), if on { ON_TEXT } else { TEXT });
    }
    ui.painter().text(
        pos2(cell.center().x, c.bottom() - 4.0),
        Align2::CENTER_BOTTOM,
        s.format_param(P::Chorus).to_uppercase(),
        small(10.0),
        if mode == 0 { DIM } else { ACCENT },
    );
}

// ---------------------------------------------------------------------------
// Bottom strip
// ---------------------------------------------------------------------------

fn strip_sections(bottom: Rect, widths: &[usize]) -> Vec<Rect> {
    let mut x = bottom.left();
    let mut out: Vec<Rect> = widths
        .iter()
        .map(|&cols| {
            let r = Rect::from_min_size(pos2(x, bottom.top()), vec2(section_width(cols), bottom.height()));
            x += r.width() + GAP;
            r
        })
        .collect();
    // Last section stretches to the right edge.
    if let Some(last) = out.last_mut() {
        last.max.x = bottom.right();
    }
    out
}

fn knob_cell(c: Rect, i: usize) -> Rect {
    col(c, i, c.top(), c.height())
}

fn tall_cell(c: Rect, i: usize) -> Rect {
    knob_cell(c, i).shrink2(vec2(4.0, 6.0))
}

fn performance_strip(ui: &mut egui::Ui, s: &Ctx, bottom: Rect) {
    let secs = strip_sections(bottom, &[4, 4, 5, 3]);

    let c = section(ui, secs[0], "KEYS");
    selector(ui, s, tall_cell(c, 0), P::KeyMode, &["MONO", "POLY 1", "POLY 2"], true);
    knob(ui, s, knob_cell(c, 1), P::Portamento, "PORTA");
    toggle(ui, s, tall_cell(c, 2), P::Hold, "HOLD");
    knob(ui, s, knob_cell(c, 3), P::Transpose, "TRANSP");

    let c = section(ui, secs[1], "CONTROLLER");
    knob(ui, s, knob_cell(c, 0), P::BendDco, "BEND");
    knob(ui, s, knob_cell(c, 1), P::BendVcf, "BND VCF");
    knob(ui, s, knob_cell(c, 2), P::ModLfo, "MOD LFO");
    knob(ui, s, knob_cell(c, 3), P::Tune, "TUNE");

    let c = section(ui, secs[2], "ARPEGGIO");
    toggle(ui, s, tall_cell(c, 0), P::ArpOn, "ON");
    selector(ui, s, tall_cell(c, 1), P::ArpMode, &["UP", "U&D", "DOWN"], true);
    selector(ui, s, tall_cell(c, 2), P::ArpRange, &["1", "2", "3"], true);
    knob(ui, s, knob_cell(c, 3), P::ArpRate, "RATE");
    let c4 = tall_cell(c, 4);
    let half = (c4.height() - 4.0) / 2.0;
    toggle(ui, s, Rect::from_min_size(c4.min, vec2(c4.width(), half)), P::ArpSync, "SYNC");
    value_box(ui, s, Rect::from_min_size(pos2(c4.left(), c4.top() + half + 4.0), vec2(c4.width(), half)), P::ArpDiv);

    output_section(ui, s, secs[3]);
}

fn output_section(ui: &mut egui::Ui, s: &Ctx, r: Rect) {
    let c = section(ui, r, "OUTPUT");
    let m = Rect::from_min_size(c.min, vec2(30.0, c.height()));
    meter(ui, m, s.get_meter(P::MeterL), s.get_meter(P::MeterR));
    let k = Rect::from_min_size(pos2(m.right() + 6.0, c.top()), vec2(COL, c.height()));
    knob(ui, s, k, P::Master, "MASTER");
}

fn settings_strip(ui: &mut egui::Ui, s: &Ctx, bottom: Rect) {
    let secs = strip_sections(bottom, &[8, 3]);
    let c = section(ui, secs[0], "SETTINGS");
    let label = |ui: &egui::Ui, cell: Rect, t: &str| {
        ui.painter().text(pos2(cell.center().x, cell.top()), Align2::CENTER_TOP, t, small(10.0), DIM);
    };
    let below = |cell: Rect| Rect::from_min_max(pos2(cell.left() + 4.0, cell.top() + 18.0), pos2(cell.right() - 4.0, cell.bottom() - 6.0));

    let c0 = knob_cell(c, 0);
    label(ui, c0, "VOICES");
    selector(ui, s, below(c0), P::Voices, &["6", "8", "10"], true);
    let c1 = knob_cell(c, 1);
    label(ui, c1, "OVERSMP");
    selector(ui, s, below(c1), P::Oversample, &["1x", "2x", "4x"], true);
    let c2 = knob_cell(c, 2);
    label(ui, c2, "VELOCITY");
    toggle(ui, s, below(c2), P::Velocity, "ON");
    let c3 = knob_cell(c, 3);
    label(ui, c3, "RETRIG");
    toggle(ui, s, below(c3), P::MonoRetrig, "MONO");
    knob(ui, s, knob_cell(c, 4), P::Drift, "DRIFT");
    knob(ui, s, knob_cell(c, 5), P::NoiseFloor, "NOISE");
    let c6 = knob_cell(c, 6);
    label(ui, c6, "HW RES");
    toggle(ui, s, below(c6), P::HwRes, "7-BIT");
    knob(ui, s, knob_cell(c, 7), P::ChorusHiss, "HISS");

    let c = section(ui, secs[1], "ABOUT");
    let lines = [
        "Juno-106 emulation · DSP ported from Ultramaster KR-106 (GPL-3.0)",
        "Drift: unison detune (Mono only).  HW Res: 7-bit sliders.  Hiss: chorus BBD noise.",
        "Double-click resets a control · Shift-drag for fine adjustment.",
    ];
    for (k, l) in lines.iter().enumerate() {
        ui.painter().text(pos2(c.left(), c.top() + 4.0 + k as f32 * 16.0), Align2::LEFT_TOP, *l, small(10.0), DIM);
    }
    output_section_compact(ui, s, Rect::from_min_max(pos2(c.right() - 100.0, c.top()), c.max));
}

fn output_section_compact(ui: &mut egui::Ui, s: &Ctx, c: Rect) {
    let m = Rect::from_min_size(pos2(c.right() - 30.0, c.top()), vec2(30.0, c.height()));
    meter(ui, m, s.get_meter(P::MeterL), s.get_meter(P::MeterR));
}
