//! The preset bank: compiled-in presets plus the user's saved ones.
//!
//! The library lives inside the plugin, not in the host's preset list.
//! `bank/` is compiled into the binary by `build.rs`; user presets go
//! through truce's [`PresetStore`], one directory shared by every format.
//!
//! A preset is a sound, not a machine. What a real 106 keeps out of its
//! patch memory stays put when a preset loads: the Settings page, master
//! volume, tuning, transpose and the bender and mod-wheel amounts. A
//! parameter a preset does not name goes to its default, so nothing leaks
//! from the sound before it.

use truce_core::presets::{PresetError, PresetRef, PresetScope, PresetStore};
use truce_core::state::hash_plugin_id;
use truce_params::ParamInfo;
use truce_utils::preset::PresetMeta;

use crate::Youth106ParamsParamId as P;

/// A compiled-in preset: `params` are `(id, plain value)` as in the file.
pub struct Preset {
    pub category: &'static str,
    pub name: &'static str,
    pub comment: &'static str,
    pub params: &'static [(u32, f64)],
}

include!(concat!(env!("OUT_DIR"), "/bank.rs"));

/// Category saved presets land in.
pub const USER_CATEGORY: &str = "User";

fn is_setting(id: u32) -> bool {
    [
        P::BendDco,
        P::BendVcf,
        P::ModLfo,
        P::Transpose,
        P::Tune,
        P::Master,
        P::Voices,
        P::Oversample,
        P::Velocity,
        P::Drift,
        P::MonoRetrig,
        P::NoiseFloor,
        P::HwRes,
        P::MidiBend,
        P::MidiMod,
        P::MidiSustain,
    ]
    .into_iter()
    .any(|p| u32::from(p) == id)
}

/// The sweep a preset makes over the panel: `(id, normalized)` for every
/// parameter it owns.
pub fn panel_targets(preset: &[(u32, f64)], infos: &[ParamInfo]) -> Vec<(u32, f64)> {
    infos
        .iter()
        .filter(|info| !is_setting(info.id))
        .map(|info| {
            let named = preset.iter().find(|(id, _)| *id == info.id).map(|(_, plain)| *plain);
            (info.id, info.range.normalize(named.unwrap_or(info.default_plain)))
        })
        .collect()
}

/// What a saved preset records: the plain value of every parameter it
/// owns, read through `normalized(id)`.
pub fn panel_snapshot(infos: &[ParamInfo], normalized: impl Fn(u32) -> f64) -> Vec<(u32, f64)> {
    infos
        .iter()
        .filter(|info| !is_setting(info.id))
        .map(|info| (info.id, info.range.denormalize(normalized(info.id))))
        .collect()
}

pub fn user_store() -> PresetStore {
    let info = truce::prelude::plugin_info!();
    PresetStore::new(info.vendor, info.name, hash_plugin_id(info.clap_id), info.preset_user_dir)
}

/// The user's own presets, in browser order.
pub fn user_presets(store: &PresetStore) -> Vec<PresetRef> {
    store.enumerate().into_iter().filter(|p| p.scope == PresetScope::User).collect()
}

/// Saves into the User category, replacing a user preset of the same name.
pub fn save_user(store: &PresetStore, name: &str, params: &[(u32, f64)]) -> Result<PresetRef, PresetError> {
    let meta = PresetMeta { name: name.to_string(), category: USER_CATEGORY.to_string(), ..PresetMeta::default() };
    store.save(meta, params, &[])
}
