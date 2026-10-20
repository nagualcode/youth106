//! Ableton-flavoured widgets bound to truce parameters.
//!
//! All continuous widgets use the host edit-gesture protocol
//! (`begin_edit` / `set_param` / `end_edit`) so DAW automation recording
//! captures a drag as one gesture. Common affordances:
//! - drag (vertical) to change, Shift for fine control
//! - double-click to reset to the default
//! - mouse wheel for small steps

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};
use truce_core::editor::{PluginContext, PluginContextReadF32};
use truce_params::Params;

use crate::Youth106Params;

pub mod theme {
    use egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1b);
    pub const PANEL: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
    pub const PANEL_LINE: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
    pub const HEADER: Color32 = Color32::from_rgb(0x32, 0x32, 0x32);
    pub const TEXT: Color32 = Color32::from_rgb(0xd6, 0xd6, 0xd6);
    pub const DIM: Color32 = Color32::from_rgb(0x8e, 0x8e, 0x8e);
    pub const TRACK: Color32 = Color32::from_rgb(0x15, 0x15, 0x15);
    pub const CONTROL: Color32 = Color32::from_rgb(0x3d, 0x3d, 0x3d);
    pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
    pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x9a, 0x3c);
    pub const ON: Color32 = Color32::from_rgb(0xff, 0xb8, 0x4d);
    pub const ON_TEXT: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1b);
    pub const JUNO_RED: Color32 = Color32::from_rgb(0xd8, 0x3b, 0x2b);
    pub const METER_LO: Color32 = Color32::from_rgb(0x5f, 0xd0, 0x6a);
    pub const METER_MID: Color32 = Color32::from_rgb(0xf0, 0xd2, 0x4a);
    pub const METER_HI: Color32 = Color32::from_rgb(0xff, 0x55, 0x45);
}
use theme::*;

pub type Ctx = PluginContext<Youth106Params>;

#[derive(Clone, Copy)]
struct Meta {
    default_norm: f32,
    steps: Option<u32>,
    bipolar: bool,
}

fn meta(state: &Ctx, id: u32) -> Meta {
    static CACHE: OnceLock<HashMap<u32, Meta>> = OnceLock::new();
    let map = CACHE.get_or_init(|| {
        state
            .params()
            .param_infos()
            .iter()
            .map(|i| {
                let min = i.range.denormalize(0.0);
                (
                    i.id,
                    Meta {
                        default_norm: i.range.normalize(i.default_plain) as f32,
                        steps: i.range.step_count().map(|s| s.get()),
                        bipolar: min < 0.0,
                    },
                )
            })
            .collect()
    });
    map.get(&id).copied().unwrap_or(Meta { default_norm: 0.0, steps: None, bipolar: false })
}

pub fn small(size: f32) -> FontId {
    FontId::proportional(size)
}

fn snap(v: f32, steps: Option<u32>) -> f32 {
    match steps {
        Some(n) if n > 0 => (v * n as f32).round() / n as f32,
        _ => v,
    }
}

/// Shared drag/wheel/reset behaviour for continuous controls. `travel` is
/// the pixel distance for a full sweep.
fn drive(ui: &egui::Ui, state: &Ctx, resp: &egui::Response, id: u32, travel: f32) -> f32 {
    let m = meta(state, id);
    let mut value = state.get_param(id);
    let acc_id = resp.id.with("acc");

    if resp.double_clicked() {
        state.automate(id, f64::from(m.default_norm));
        return m.default_norm;
    }
    if resp.drag_started() {
        state.begin_edit(id);
        ui.memory_mut(|mem| mem.data.insert_temp(acc_id, value));
    }
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let scale = if fine { 0.1 } else { 1.0 };
        let delta = -resp.drag_delta().y / travel * scale;
        let raw: f32 = ui.memory(|mem| mem.data.get_temp(acc_id)).unwrap_or(value);
        let raw = (raw + delta).clamp(0.0, 1.0);
        ui.memory_mut(|mem| mem.data.insert_temp(acc_id, raw));
        let snapped = snap(raw, m.steps);
        if (snapped - value).abs() > 1e-6 {
            value = snapped;
            state.set_param(id, f64::from(value));
        }
    }
    if resp.drag_stopped() {
        state.end_edit(id);
    }
    if resp.hovered() && !resp.dragged() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.5 {
            let step = match m.steps {
                Some(n) if n > 0 => scroll.signum() / n as f32,
                _ => scroll / 1500.0,
            };
            value = (value + step).clamp(0.0, 1.0);
            state.automate(id, f64::from(value));
        }
    }
    value
}

/// Vertical Juno-style fader with Ableton flat styling.
pub fn fader(ui: &mut egui::Ui, state: &Ctx, cell: Rect, id: impl Into<u32>, label: &str) {
    let id = id.into();
    let m = meta(state, id);
    let p = ui.painter().clone();
    p.text(pos2(cell.center().x, cell.top()), Align2::CENTER_TOP, label, small(10.0), DIM);

    let track = Rect::from_center_size(
        pos2(cell.center().x, (cell.top() + 16.0 + cell.bottom() - 16.0) * 0.5),
        vec2(6.0, cell.height() - 40.0),
    );
    let hit = track.expand2(vec2(14.0, 6.0));
    let resp = ui.interact(hit, ui.id().with(("fader", id)), Sense::click_and_drag());
    let v = drive(ui, state, &resp, id, track.height());

    // Tick marks 0..10 like the Juno panel.
    for k in 0..=10 {
        let y = track.bottom() - track.height() * k as f32 / 10.0;
        let w = if k % 5 == 0 { 7.0 } else { 4.0 };
        p.line_segment(
            [pos2(track.left() - 4.0 - w, y), pos2(track.left() - 4.0, y)],
            Stroke::new(1.0_f32, PANEL_LINE),
        );
    }
    if let Some(n) = m.steps {
        for k in 0..=n {
            let y = track.bottom() - track.height() * k as f32 / n as f32;
            p.line_segment([pos2(track.right() + 4.0, y), pos2(track.right() + 9.0, y)], Stroke::new(1.0_f32, DIM));
        }
    }

    p.rect_filled(track, CornerRadius::same(3), TRACK);
    let fill_top = track.bottom() - track.height() * v;
    let fill = Rect::from_min_max(pos2(track.left(), fill_top), track.max);
    p.rect_filled(fill, CornerRadius::same(3), ACCENT.gamma_multiply(0.85));

    let active = resp.hovered() || resp.dragged();
    let cap = Rect::from_center_size(pos2(track.center().x, fill_top), vec2(24.0, 9.0));
    p.rect_filled(cap, CornerRadius::same(2), if active { Color32::WHITE } else { TEXT });
    p.line_segment([pos2(cap.left() + 3.0, cap.center().y), pos2(cap.right() - 3.0, cap.center().y)], Stroke::new(1.5_f32, ACCENT));

    let text = state.format_param(id);
    let col = if active { ACCENT } else { TEXT };
    p.text(pos2(cell.center().x, cell.bottom()), Align2::CENTER_BOTTOM, text, small(10.0), col);
}

/// Ableton-style rotary dial.
pub fn knob(ui: &mut egui::Ui, state: &Ctx, cell: Rect, id: impl Into<u32>, label: &str) {
    let id = id.into();
    let m = meta(state, id);
    let p = ui.painter().clone();
    p.text(pos2(cell.center().x, cell.top()), Align2::CENTER_TOP, label, small(10.0), DIM);

    let r = 14.0;
    let c = pos2(cell.center().x, cell.top() + 16.0 + r + 3.0);
    let hit = Rect::from_center_size(c, Vec2::splat(2.0 * r + 10.0));
    let resp = ui.interact(hit, ui.id().with(("knob", id)), Sense::click_and_drag());
    let v = drive(ui, state, &resp, id, 160.0);

    const START: f32 = std::f32::consts::FRAC_PI_4 * 3.0;
    const SWEEP: f32 = std::f32::consts::FRAC_PI_2 * 3.0;
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let n = 40;
        (0..=n)
            .map(|i| {
                let a = START + SWEEP * (from + (to - from) * i as f32 / n as f32);
                c + vec2(a.cos(), a.sin()) * r
            })
            .collect()
    };
    p.add(Shape::line(arc(0.0, 1.0), Stroke::new(3.0_f32, TRACK)));
    let base = if m.bipolar { 0.5 } else { 0.0 };
    if (v - base).abs() > 0.002 {
        p.add(Shape::line(arc(base.min(v), base.max(v)), Stroke::new(3.0_f32, ACCENT)));
    }
    let active = resp.hovered() || resp.dragged();
    let a = START + SWEEP * v;
    p.circle_filled(c, r - 5.0, if active { CONTROL_HOVER } else { CONTROL });
    p.line_segment([c + vec2(a.cos(), a.sin()) * 3.0, c + vec2(a.cos(), a.sin()) * (r - 4.0)], Stroke::new(2.0_f32, TEXT));

    let text = state.format_param(id);
    p.text(pos2(cell.center().x, cell.bottom()), Align2::CENTER_BOTTOM, text, small(10.0), if active { ACCENT } else { TEXT });
}

fn button_face(p: &egui::Painter, rect: Rect, on: bool, hovered: bool, text: &str, size: f32) {
    let fill = if on {
        ON
    } else if hovered {
        CONTROL_HOVER
    } else {
        CONTROL
    };
    p.rect_filled(rect, CornerRadius::same(2), fill);
    p.text(rect.center(), Align2::CENTER_CENTER, text, small(size), if on { ON_TEXT } else { TEXT });
}

/// Latching on/off button for a `BoolParam`.
pub fn toggle(ui: &mut egui::Ui, state: &Ctx, rect: Rect, id: impl Into<u32>, text: &str) {
    let id = id.into();
    let on = state.get_param(id) > 0.5;
    let resp = ui.interact(rect, ui.id().with(("toggle", id)), Sense::click());
    if resp.clicked() {
        state.automate(id, if on { 0.0 } else { 1.0 });
    }
    button_face(ui.painter(), rect, on, resp.hovered(), text, 10.0);
}

/// Segmented selector for an `EnumParam`. `labels` are in index order;
/// when vertical, index 0 is drawn at the bottom (like a panel switch).
pub fn selector(
    ui: &mut egui::Ui,
    state: &Ctx,
    rect: Rect,
    id: impl Into<u32>,
    labels: &[&str],
    vertical: bool,
) {
    let id = id.into();
    let n = labels.len();
    let cur = (state.get_param(id) * (n as f32 - 1.0)).round() as usize;
    let gap = 2.0;
    for (i, label) in labels.iter().enumerate() {
        let seg = if vertical {
            let h = (rect.height() - gap * (n as f32 - 1.0)) / n as f32;
            let top = rect.bottom() - (i as f32 + 1.0) * h - i as f32 * gap;
            Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), h))
        } else {
            let w = (rect.width() - gap * (n as f32 - 1.0)) / n as f32;
            Rect::from_min_size(pos2(rect.left() + i as f32 * (w + gap), rect.top()), vec2(w, rect.height()))
        };
        let resp = ui.interact(seg, ui.id().with(("sel", id, i)), Sense::click());
        if resp.clicked() && i != cur {
            state.automate(id, i as f64 / (n as f64 - 1.0));
        }
        button_face(ui.painter(), seg, i == cur, resp.hovered(), label, 9.5);
    }
}

/// Compact value box for long enums (divisions, voice count): drag or
/// wheel to change, click to step forward, Shift-click to step back.
pub fn value_box(ui: &mut egui::Ui, state: &Ctx, rect: Rect, id: impl Into<u32>) {
    let id = id.into();
    let m = meta(state, id);
    let n = m.steps.unwrap_or(1).max(1);
    let resp = ui.interact(rect, ui.id().with(("vbox", id)), Sense::click_and_drag());
    let cur = state.get_param(id);
    if resp.clicked() {
        let back = ui.input(|i| i.modifiers.shift);
        let idx = (cur * n as f32).round() as i32 + if back { -1 } else { 1 };
        let idx = idx.rem_euclid(n as i32 + 1);
        state.automate(id, f64::from(idx as f32 / n as f32));
    } else {
        drive(ui, state, &resp, id, 12.0 * n as f32);
    }
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(2), TRACK);
    p.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0_f32, if resp.hovered() { ACCENT } else { PANEL_LINE }), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, state.format_param(id), small(10.0), TEXT);
}

/// Section panel with a title bar; returns the content rect.
pub fn section(ui: &egui::Ui, rect: Rect, title: &str) -> Rect {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(4), PANEL);
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 20.0));
    p.rect_filled(head, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, HEADER);
    p.text(pos2(head.left() + 8.0, head.center().y), Align2::LEFT_CENTER, title, FontId::proportional(11.0), TEXT);
    Rect::from_min_max(pos2(rect.left() + 8.0, head.bottom() + 8.0), pos2(rect.right() - 8.0, rect.bottom() - 8.0))
}

/// Stereo peak meter with dB ticks (-48..+6 dBFS).
pub fn meter(ui: &egui::Ui, rect: Rect, l: f32, r: f32) {
    let p = ui.painter();
    let db_to_y = |db: f32| {
        let t = ((db + 48.0) / 54.0).clamp(0.0, 1.0);
        rect.bottom() - t * rect.height()
    };
    let bar_w = (rect.width() - 14.0) / 2.0;
    for (k, level) in [l, r].into_iter().enumerate() {
        let x0 = rect.left() + k as f32 * (bar_w + 2.0);
        let bar = Rect::from_min_size(pos2(x0, rect.top()), vec2(bar_w, rect.height()));
        p.rect_filled(bar, CornerRadius::ZERO, TRACK);
        let db = 20.0 * level.max(1e-6).log10();
        let y = db_to_y(db);
        for (lo, hi, col) in [(-48.0, -12.0, METER_LO), (-12.0, -3.0, METER_MID), (-3.0, 6.0, METER_HI)] {
            let top = db_to_y(hi).max(y);
            let bottom = db_to_y(lo);
            if top < bottom {
                p.rect_filled(Rect::from_min_max(pos2(x0, top), pos2(x0 + bar_w, bottom)), CornerRadius::ZERO, col);
            }
        }
    }
    for db in [0.0, -12.0, -24.0, -36.0] {
        let y = db_to_y(db);
        p.line_segment([pos2(rect.right() - 10.0, y), pos2(rect.right() - 6.0, y)], Stroke::new(1.0_f32, DIM));
    }
}
