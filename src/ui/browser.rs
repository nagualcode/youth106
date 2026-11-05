//! The preset browser: a bar in the header (previous, next, current name,
//! save) and a page that replaces the panel while it is open. Clicking a
//! preset loads it and keeps the page open, so sounds can be auditioned
//! from the keyboard one after another.

use egui::{Align2, Color32, CornerRadius, Key, Rect, ScrollArea, Sense, Stroke, TextEdit, UiBuilder, pos2, vec2};
use truce_core::editor::PluginContextReadF64;
use truce_core::presets::{PresetRef, PresetStore};
use truce_params::{ParamInfo, Params};

use super::widgets::theme::{ACCENT, CONTROL, CONTROL_HOVER, DIM, METER_HI, ON, ON_TEXT, PANEL, TEXT, TRACK};
use super::widgets::{Ctx, action_button, button_face, small};
use crate::bank::{self, BANK, Preset, USER_CATEGORY};

const SIDEBAR_W: f32 = 150.0;
const CAT_H: f32 = 24.0;
const ROW_H: f32 = 24.0;
const COLUMNS: usize = 4;
const FOOTER_H: f32 = 30.0;
const ALL: &str = "ALL";

enum Source {
    Bank(&'static Preset),
    User(PresetRef),
}

struct Entry {
    category: String,
    name: String,
    comment: String,
    source: Source,
}

impl Entry {
    fn key(&self) -> String {
        format!("{}/{}", self.category, self.name)
    }

    fn matches(&self, category: &str, search: &str) -> bool {
        (category == ALL || category == self.category)
            && (search.is_empty() || self.name.to_lowercase().contains(search) || self.comment.to_lowercase().contains(search))
    }
}

pub struct Browser {
    pub open: bool,
    store: PresetStore,
    infos: Vec<ParamInfo>,
    entries: Vec<Entry>,
    categories: Vec<String>,
    category: String,
    search: String,
    /// Name being typed in the save row.
    saving: Option<String>,
    focus_name: bool,
    /// User preset waiting for the delete confirmation, by key.
    deleting: Option<String>,
    status: String,
}

impl Browser {
    pub fn new() -> Self {
        let mut categories: Vec<String> = vec![ALL.to_string()];
        for p in BANK {
            if !categories.iter().any(|c| c == p.category) {
                categories.push(p.category.to_string());
            }
        }
        categories.push(USER_CATEGORY.to_string());
        Self {
            open: false,
            store: bank::user_store(),
            infos: crate::Youth106Params::default().param_infos(),
            entries: Vec::new(),
            categories,
            category: ALL.to_string(),
            search: String::new(),
            saving: None,
            focus_name: false,
            deleting: None,
            status: String::new(),
        }
    }

    fn rescan(&mut self) {
        self.entries.clear();
        self.entries.extend(BANK.iter().map(|p| Entry {
            category: p.category.to_string(),
            name: p.name.to_string(),
            comment: p.comment.to_string(),
            source: Source::Bank(p),
        }));
        self.entries.extend(bank::user_presets(&self.store).into_iter().map(|p| Entry {
            category: USER_CATEGORY.to_string(),
            name: p.name.clone(),
            comment: p.comment.clone().unwrap_or_default(),
            source: Source::User(p),
        }));
    }

    fn current_key(s: &Ctx) -> String {
        s.params().current_preset.lock().map(|k| k.clone()).unwrap_or_default()
    }

    fn set_current(s: &Ctx, key: String) {
        if let Ok(mut k) = s.params().current_preset.lock() {
            *k = key;
        }
    }

    fn load(&mut self, s: &Ctx, index: usize) {
        let Some(entry) = self.entries.get(index) else { return };
        let params = match &entry.source {
            Source::Bank(p) => p.params.to_vec(),
            Source::User(p) => match self.store.load(&p.uuid) {
                Ok(state) => state.params,
                Err(e) => {
                    self.status = format!("Could not load {}: {e}", entry.name);
                    return;
                }
            },
        };
        // One gesture, so a host that records it sees a single edit.
        let changes: Vec<(u32, f64)> =
            bank::panel_targets(&params, &self.infos).into_iter().filter(|(id, n)| (s.get_param(*id) - n).abs() > 1e-7).collect();
        for (id, _) in &changes {
            s.begin_edit(*id);
        }
        for (id, n) in &changes {
            s.set_param(*id, *n);
        }
        for (id, _) in &changes {
            s.end_edit(*id);
        }
        Self::set_current(s, entry.key());
        self.status.clear();
    }

    /// Previous / next within the category and search on the page.
    fn step(&mut self, s: &Ctx, delta: isize) {
        if self.entries.is_empty() {
            self.rescan();
        }
        let search = self.search.to_lowercase();
        let visible: Vec<usize> = (0..self.entries.len()).filter(|&i| self.entries[i].matches(&self.category, &search)).collect();
        if visible.is_empty() {
            return;
        }
        let key = Self::current_key(s);
        let at = visible.iter().position(|&i| self.entries[i].key() == key);
        let len = visible.len() as isize;
        let next = at.map_or(if delta > 0 { 0 } else { len - 1 }, |a| (a as isize + delta).rem_euclid(len));
        self.load(s, visible[next as usize]);
    }

    fn save(&mut self, s: &Ctx, name: &str) {
        let params = bank::panel_snapshot(&self.infos, |id| s.get_param(id));
        match bank::save_user(&self.store, name, &params) {
            Ok(saved) => {
                Self::set_current(s, format!("{USER_CATEGORY}/{}", saved.name));
                self.status = format!("Saved {}", saved.name);
                self.category = USER_CATEGORY.to_string();
                self.saving = None;
                self.rescan();
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    fn delete(&mut self, key: &str) {
        let Some(Entry { source: Source::User(p), name, .. }) = self.entries.iter().find(|e| e.key() == key) else { return };
        self.status = match self.store.delete(&p.uuid) {
            Ok(()) => format!("Deleted {name}"),
            Err(e) => format!("Could not delete {name}: {e}"),
        };
        self.deleting = None;
        self.rescan();
    }

    fn start_saving(&mut self, s: &Ctx) {
        let key = Self::current_key(s);
        let name = key.split_once('/').map_or("", |(_, n)| n);
        self.saving = Some(if name.is_empty() { "New Preset".to_string() } else { name.to_string() });
        self.focus_name = true;
    }

    /// `◀ ▶ [ name ▾ ] SAVE` in the header.
    pub fn bar(&mut self, ui: &mut egui::Ui, s: &Ctx, r: Rect) {
        const STEP: f32 = 22.0;
        const SAVE: f32 = 52.0;
        let prev = Rect::from_min_size(r.min, vec2(STEP, r.height()));
        let next = prev.translate(vec2(STEP + 2.0, 0.0));
        if action_button(ui, prev, "preset-prev", "<", ACCENT) {
            self.step(s, -1);
        }
        if action_button(ui, next, "preset-next", ">", ACCENT) {
            self.step(s, 1);
        }

        let save = Rect::from_min_size(pos2(r.right() - SAVE, r.top()), vec2(SAVE, r.height()));
        if action_button(ui, save, "preset-save", "SAVE", ACCENT) {
            self.open_page();
            self.start_saving(s);
        }

        let name = Rect::from_min_max(pos2(next.right() + 2.0, r.top()), pos2(save.left() - 2.0, r.bottom()));
        let resp = ui.interact(name, ui.id().with("preset-name"), Sense::click());
        if resp.clicked() {
            if self.open {
                self.open = false;
            } else {
                self.open_page();
            }
        }
        let p = ui.painter();
        p.rect_filled(name, CornerRadius::same(2), if self.open { CONTROL_HOVER } else { TRACK });
        if resp.hovered() {
            p.rect_stroke(name, CornerRadius::same(2), Stroke::new(1.0_f32, ACCENT), egui::StrokeKind::Inside);
        }
        let key = Self::current_key(s);
        let (label, color) = match key.split_once('/') {
            Some((_, n)) => (n, TEXT),
            None => ("Init", DIM),
        };
        p.text(pos2(name.left() + 8.0, name.center().y), Align2::LEFT_CENTER, label, small(11.0), color);
        p.text(pos2(name.right() - 8.0, name.center().y), Align2::RIGHT_CENTER, if self.open { "CLOSE" } else { "BROWSE" }, small(8.5), DIM);
    }

    fn open_page(&mut self) {
        self.open = true;
        self.saving = None;
        self.deleting = None;
        self.status.clear();
        self.rescan();
    }

    /// The page shown instead of the panel.
    pub fn page(&mut self, ui: &mut egui::Ui, s: &Ctx, r: Rect) {
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            if self.saving.is_some() || self.deleting.is_some() {
                self.saving = None;
                self.deleting = None;
            } else {
                self.open = false;
            }
        }
        ui.painter().rect_filled(r, CornerRadius::same(3), PANEL);

        let inner = r.shrink(10.0);
        let sidebar = Rect::from_min_size(inner.min, vec2(SIDEBAR_W, inner.height()));
        self.sidebar(ui, sidebar);

        let main = Rect::from_min_max(pos2(sidebar.right() + 10.0, inner.top()), inner.max);
        let search = Rect::from_min_size(main.min, vec2(main.width(), 24.0));
        ui.scope_builder(UiBuilder::new().max_rect(search), |ui| {
            ui.add_sized(search.size(), TextEdit::singleline(&mut self.search).hint_text("Search name, artist, song, synth…"));
        });

        let footer = Rect::from_min_max(pos2(main.left(), main.bottom() - FOOTER_H), main.max);
        let list = Rect::from_min_max(pos2(main.left(), search.bottom() + 8.0), pos2(main.right(), footer.top() - 8.0));
        let hovered = self.list(ui, s, list);
        self.footer(ui, s, footer, hovered);
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, r: Rect) {
        for (i, category) in self.categories.clone().iter().enumerate() {
            let b = Rect::from_min_size(pos2(r.left(), r.top() + i as f32 * (CAT_H + 3.0)), vec2(r.width(), CAT_H));
            let resp = ui.interact(b, ui.id().with(("preset-cat", i)), Sense::click());
            if resp.clicked() {
                self.category.clone_from(category);
            }
            button_face(ui.painter(), b, *category == self.category, resp.hovered(), &category.to_uppercase(), 10.0);
        }
    }

    /// Returns the entry under the mouse.
    fn list(&mut self, ui: &mut egui::Ui, s: &Ctx, r: Rect) -> Option<usize> {
        let search = self.search.to_lowercase();
        let visible: Vec<usize> = (0..self.entries.len()).filter(|&i| self.entries[i].matches(&self.category, &search)).collect();
        let current = Self::current_key(s);
        let mut hovered = None;
        let mut load = None;
        let mut delete = None;

        ui.scope_builder(UiBuilder::new().max_rect(r), |ui| {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let width = ui.available_width() - 12.0;
                let cell_w = width / COLUMNS as f32;
                let rows = visible.len().div_ceil(COLUMNS);
                let (area, _) = ui.allocate_exact_size(vec2(width, rows as f32 * ROW_H), Sense::hover());
                for (n, &i) in visible.iter().enumerate() {
                    let entry = &self.entries[i];
                    let cell = Rect::from_min_size(
                        pos2(area.left() + (n % COLUMNS) as f32 * cell_w, area.top() + (n / COLUMNS) as f32 * ROW_H),
                        vec2(cell_w - 3.0, ROW_H - 3.0),
                    );
                    let id = ui.id().with(("preset", i));
                    let resp = ui.interact(cell, id, Sense::click());
                    let on = entry.key() == current;
                    let user = matches!(entry.source, Source::User(_));
                    if resp.hovered() {
                        hovered = Some(i);
                    }
                    let fill: Color32 = if on {
                        ON
                    } else if resp.hovered() {
                        CONTROL_HOVER
                    } else {
                        CONTROL
                    };
                    ui.painter().rect_filled(cell, CornerRadius::same(2), fill);
                    let color = if on { ON_TEXT } else { TEXT };
                    ui.painter().text(pos2(cell.left() + 8.0, cell.center().y), Align2::LEFT_CENTER, &entry.name, small(11.0), color);

                    let del = Rect::from_min_size(pos2(cell.right() - cell.height(), cell.top()), vec2(cell.height(), cell.height()));
                    let over_delete = user && resp.hover_pos().is_some_and(|p| del.contains(p));
                    if user && resp.hovered() {
                        ui.painter().text(del.center(), Align2::CENTER_CENTER, "×", small(13.0), if over_delete { METER_HI } else { color });
                    }
                    if resp.clicked() {
                        if over_delete {
                            delete = Some(entry.key());
                        } else {
                            load = Some(i);
                        }
                    }
                }
                if visible.is_empty() {
                    let text = if self.category == USER_CATEGORY && self.search.is_empty() { "No saved presets yet: press SAVE." } else { "Nothing matches." };
                    ui.label(text);
                }
            });
        });
        if let Some(i) = load {
            self.load(s, i);
        }
        if delete.is_some() {
            self.deleting = delete;
            self.saving = None;
        }
        hovered
    }

    fn footer(&mut self, ui: &mut egui::Ui, s: &Ctx, r: Rect, hovered: Option<usize>) {
        const BTN: f32 = 90.0;
        let center = r.center().y;
        let second = Rect::from_min_size(pos2(r.right() - BTN, r.top() + 4.0), vec2(BTN, r.height() - 8.0));
        let first = second.translate(vec2(-(BTN + 6.0), 0.0));

        if let Some(mut name) = self.saving.take() {
            ui.painter().text(pos2(r.left(), center), Align2::LEFT_CENTER, "SAVE AS", small(10.0), DIM);
            let field = Rect::from_min_max(pos2(r.left() + 56.0, r.top() + 3.0), pos2(first.left() - 8.0, r.bottom() - 3.0));
            let mut enter = false;
            ui.scope_builder(UiBuilder::new().max_rect(field), |ui| {
                let resp = ui.add_sized(field.size(), TextEdit::singleline(&mut name));
                if std::mem::take(&mut self.focus_name) {
                    resp.request_focus();
                }
                enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            });
            let clean = name.trim().to_string();
            let exists = self.entries.iter().any(|e| matches!(e.source, Source::User(_)) && e.name == clean);
            let text = if exists { "OVERWRITE" } else { "SAVE" };
            let confirm = action_button(ui, first, "preset-confirm", text, ACCENT);
            let cancel = action_button(ui, second, "preset-cancel", "CANCEL", ACCENT);
            self.saving = Some(name);
            if cancel {
                self.saving = None;
            } else if (confirm || enter) && !clean.is_empty() {
                self.save(s, &clean);
            }
        } else if let Some(key) = self.deleting.clone() {
            let name = key.split_once('/').map_or(key.as_str(), |(_, n)| n);
            ui.painter().text(pos2(r.left(), center), Align2::LEFT_CENTER, format!("Delete {name}? This removes the file."), small(11.0), METER_HI);
            if action_button(ui, first, "preset-delete", "DELETE", METER_HI) {
                self.delete(&key);
            }
            if action_button(ui, second, "preset-keep", "KEEP", ACCENT) {
                self.deleting = None;
            }
        } else {
            let key = Self::current_key(s);
            let entry = hovered.and_then(|i| self.entries.get(i)).or_else(|| self.entries.iter().find(|e| e.key() == key));
            let text = if !self.status.is_empty() { self.status.as_str() } else { entry.map_or("", |e| e.comment.as_str()) };
            let clipped = ui.painter().with_clip_rect(r);
            clipped.text(pos2(r.left(), center), Align2::LEFT_CENTER, text, small(11.0), DIM);
        }
    }
}
