//! Compiles `bank/**/*.preset` into the plugin binary.
//!
//! The presets live inside the plugin, browsed from its own interface, so
//! they are deliberately not in a `presets/` directory: `cargo truce
//! install` would export that to the host's preset list. The sub-directory
//! is the category; a file at the root of `bank/` is category "Init".

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

/// Categories in browser order; any other directory follows, sorted.
const ORDER: [&str; 7] = ["Init", "Juno-106 A", "Juno-106 B", "Juno-60 A", "Juno-60 B", "Classics", "Sound Design"];

struct Entry {
    rank: usize,
    category: String,
    name: String,
    comment: String,
    params: BTreeMap<u32, f64>,
}

fn preset_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("reading bank/") {
        let path = entry.expect("reading bank/").path();
        if path.is_dir() {
            preset_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "preset") {
            out.push(path);
        }
    }
}

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("bank");
    println!("cargo:rerun-if-changed=bank");

    let mut files = Vec::new();
    preset_files(&root, &mut files);

    let mut presets = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).expect("reading preset");
        let doc: toml::Table = text.parse().unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let name = doc["name"].as_str().expect("preset name").to_string();
        let comment = doc.get("comment").and_then(toml::Value::as_str).unwrap_or_default().to_string();
        let category = match path.parent().expect("parent").strip_prefix(&root).expect("under bank/").to_str() {
            Some("") | None => "Init".to_string(),
            Some(dir) => dir.to_string(),
        };
        let rank = ORDER.iter().position(|c| *c == category).unwrap_or(ORDER.len());
        let mut params = BTreeMap::new();
        let table = doc.get("params").and_then(toml::Value::as_table).expect("[params]");
        for (key, value) in table {
            let id: u32 = key.parse().unwrap_or_else(|_| panic!("{}: param key {key}", path.display()));
            let plain = match value {
                toml::Value::Boolean(b) => f64::from(u8::from(*b)),
                toml::Value::Float(f) => *f,
                toml::Value::Integer(i) => *i as f64,
                other => panic!("{}: {key} = {other}", path.display()),
            };
            params.insert(id, plain);
        }
        presets.push(Entry { rank, category, name, comment, params });
    }
    presets.sort_by(|a, b| (a.rank, &a.category, &a.name).cmp(&(b.rank, &b.category, &b.name)));

    let mut code = String::from("pub static BANK: &[Preset] = &[\n");
    for Entry { category, name, comment, params, .. } in &presets {
        write!(code, "    Preset {{ category: {category:?}, name: {name:?}, comment: {comment:?}, params: &[").unwrap();
        for (id, plain) in params {
            write!(code, "({id}, {plain:?}), ").unwrap();
        }
        code.push_str("] },\n");
    }
    code.push_str("];\n");

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("bank.rs");
    fs::write(out, code).expect("writing bank.rs");
}
