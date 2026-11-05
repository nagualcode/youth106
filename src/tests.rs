use std::time::Duration;

use truce_test::{assert_no_audio_alloc, assertions, driver};

use super::*;

const SR: f64 = 48_000.0;

/// Positive-going zero crossings per second over `samples`.
fn crossing_hz(samples: &[f32], sr: f64) -> f64 {
    let n = samples.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    n as f64 * sr / samples.len() as f64
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

/// A plain test patch: saw only, filter open, chorus off, gate VCA.
fn plain(d: truce_test::PluginDriver<Plugin>) -> truce_test::PluginDriver<Plugin> {
    d.sample_rate(SR)
        .set_param(P::DcoPulse, 0.0)
        .set_param(P::DcoSaw, 1.0)
        .set_param(P::Chorus, 0.0)
        .set_param(P::VcfFreq, 1.0)
        .set_param(P::VcaMode, 1.0)
}

#[test]
fn info_is_valid() {
    truce_test::assert_valid_info::<Plugin>();
}

#[test]
fn bus_config_instrument() {
    truce_test::assert_bus_config_instrument::<Plugin>();
}

#[test]
fn params_are_sane() {
    truce_test::assert_param_defaults_match::<Plugin>();
    truce_test::assert_param_normalized_clamped::<Plugin>();
    truce_test::assert_param_normalized_roundtrip::<Plugin>();
    truce_test::assert_no_duplicate_param_ids::<Plugin>();
}

#[test]
fn state_round_trips() {
    truce_test::assert_state_round_trip::<Plugin>();
    truce_test::assert_corrupt_state_no_crash::<Plugin>();
    truce_test::assert_empty_state_no_crash::<Plugin>();
}

#[test]
fn has_editor() {
    truce_test::assert_has_editor::<Plugin>();
}

#[test]
fn idle_output_is_only_the_noise_floor() {
    let r = driver!(Plugin).sample_rate(SR).duration(Duration::from_millis(300)).run();
    assertions::assert_no_nans(&r);
    // Analog floor sits ~70 dB below a playing voice.
    let level = rms(&r.output[0][4800..]);
    assert!(level < 1e-3, "idle rms {level}");
}

#[test]
fn note_produces_sound() {
    let r = driver!(Plugin)
        .sample_rate(SR)
        .duration(Duration::from_millis(200))
        .script(|s| s.note_on(60, 0.8))
        .run();
    assertions::assert_no_nans(&r);
    let level = rms(&r.output[0][4800..]);
    assert!(level > 0.02, "note rms {level}");
}

#[test]
fn a4_is_440_hz() {
    let r = plain(driver!(Plugin))
        .duration(Duration::from_millis(1100))
        .script(|s| s.note_on(69, 0.8))
        .run();
    let hz = crossing_hz(&r.output[0][4800..], SR);
    assert!((hz - 440.0).abs() < 2.0, "measured {hz} Hz");
}

#[test]
fn range_switch_shifts_octave() {
    // 16' = index 0 of 3.
    let r = plain(driver!(Plugin))
        .set_param(P::DcoRange, 0.0)
        .duration(Duration::from_millis(1100))
        .script(|s| s.note_on(69, 0.8))
        .run();
    let hz = crossing_hz(&r.output[0][4800..], SR);
    assert!((hz - 220.0).abs() < 2.0, "measured {hz} Hz");
}

#[test]
fn pitch_bend_uses_bend_range() {
    // Default bend range is +-2 semitones.
    let r = plain(driver!(Plugin))
        .duration(Duration::from_millis(1100))
        .script(|s| {
            s.pitch_bend(1.0);
            s.note_on(69, 0.8);
        })
        .run();
    let hz = crossing_hz(&r.output[0][4800..], SR);
    let expect = 440.0 * 2f64.powf(2.0 / 12.0);
    assert!((hz - expect).abs() < 3.0, "measured {hz} Hz, want {expect}");
}

#[test]
fn note_off_releases() {
    let r = plain(driver!(Plugin))
        .duration(Duration::from_millis(600))
        .script(|s| {
            s.note_on(60, 0.8);
            s.wait_ms(200);
            s.note_off(60);
        })
        .run();
    let tail = rms(&r.output[0][r.output[0].len() - 4800..]);
    assert!(tail < 1e-3, "tail rms {tail}");
}

#[test]
fn sustain_pedal_holds_notes() {
    let r = plain(driver!(Plugin))
        .duration(Duration::from_millis(600))
        .script(|s| {
            s.cc(64, 1.0);
            s.note_on(60, 0.8);
            s.wait_ms(100);
            s.note_off(60);
        })
        .run();
    let tail = rms(&r.output[0][r.output[0].len() - 4800..]);
    assert!(tail > 0.02, "held tail rms {tail}");
}

#[test]
fn chorus_widens_stereo() {
    let r = driver!(Plugin)
        .sample_rate(SR)
        .set_param(P::Chorus, 1.0 / 3.0)
        .duration(Duration::from_millis(500))
        .script(|s| s.note_on(60, 0.8))
        .run();
    let diff: Vec<f32> =
        r.output[0].iter().zip(&r.output[1]).map(|(l, r)| l - r).collect();
    assert!(rms(&diff[9600..]) > 0.005, "chorus should decorrelate L/R");
}

#[test]
fn chorus_adds_no_hiss() {
    // Idle: above ~1 kHz the chorus adds almost nothing to the dry floor
    // (the BBD hiss is not modeled).
    let hf_rms = |chorus: f64| {
        let r = driver!(Plugin).sample_rate(SR).set_param(P::Chorus, chorus).duration(Duration::from_millis(2000)).run();
        let hf: Vec<f32> = r.output[0].windows(2).map(|w| w[1] - w[0]).collect();
        rms(&hf[9600..])
    };
    let (off, on) = (hf_rms(0.0), hf_rms(1.0 / 3.0));
    assert!(on < 5e-5, "chorus hf noise {on} (off {off})");
}

#[test]
fn every_mode_renders_cleanly() {
    // Exercise each switch position and extreme slider for NaNs/blowups.
    let r = driver!(Plugin)
        .sample_rate(SR)
        .set_param(P::VcfRes, 1.0)
        .set_param(P::VcfEnv, 1.0)
        .set_param(P::VcfLfo, 1.0)
        .set_param(P::DcoLfo, 1.0)
        .set_param(P::DcoNoise, 1.0)
        .set_param(P::DcoSub, 1.0)
        .set_param(P::PwmMode, 0.0)
        .set_param(P::Hpf, 0.0)
        .set_param(P::Chorus, 1.0)
        .set_param(P::Oversample, 1.0)
        .set_param(P::Voices, 1.0)
        .duration(Duration::from_millis(400))
        .script(|s| {
            for n in [36, 48, 60, 72, 84, 96, 108, 24, 40, 52] {
                s.note_on(n, 1.0);
            }
            s.wait_ms(150);
            s.set_param(P::KeyMode, 0.0);
            s.wait_ms(100);
            s.set_param(P::ArpOn, 1.0);
        })
        .run();
    assertions::assert_no_nans(&r);
    let peak = r.output[0].iter().fold(0.0_f32, |m, x| m.max(x.abs()));
    assert!(peak < 4.0, "peak {peak}");
}

#[test]
fn poly_drops_seventh_note_like_hardware() {
    // Six voices: a 7th simultaneous note is not stolen, it's dropped.
    let six = plain(driver!(Plugin))
        .duration(Duration::from_millis(300))
        .script(|s| {
            for n in 60..66 {
                s.note_on(n, 0.8);
            }
        })
        .run();
    let seven = plain(driver!(Plugin))
        .duration(Duration::from_millis(300))
        .script(|s| {
            for n in 60..67 {
                s.note_on(n, 0.8);
            }
        })
        .run();
    let a = rms(&six.output[0][4800..]);
    let b = rms(&seven.output[0][4800..]);
    assert!((a - b).abs() / a < 0.05, "6 voices {a} vs 7 notes {b}");
}

#[test]
fn unison_voices_are_phase_offset_not_stacked() {
    // Free-running DCOs: six unison voices at one pitch don't add up in
    // phase (6x = +15.6 dB), they sit at fixed arbitrary phase offsets.
    let one = plain(driver!(Plugin))
        .duration(Duration::from_millis(400))
        .script(|s| s.note_on(57, 0.8))
        .run();
    let uni = plain(driver!(Plugin))
        .set_param(P::KeyMode, 0.0)
        .duration(Duration::from_millis(400))
        .script(|s| s.note_on(57, 0.8))
        .run();
    let ratio = rms(&uni.output[0][9600..]) / rms(&one.output[0][9600..]);
    assert!(ratio > 1.2 && ratio < 5.0, "unison/single rms ratio {ratio}");
}

#[test]
fn saw_level_is_steady_across_the_keyboard() {
    // DCO CV compensation: in steady state the ramp height doesn't depend
    // on pitch (filter open, so the level is the oscillator's).
    let level = |note: u8| {
        let r = plain(driver!(Plugin))
            .duration(Duration::from_millis(400))
            .script(move |s| s.note_on(note, 0.8))
            .run();
        rms(&r.output[0][9600..])
    };
    let lo = level(48);
    let hi = level(84);
    assert!((lo / hi - 1.0).abs() < 0.1, "C3 {lo} vs C6 {hi}");
}

#[test]
fn hardware_resolution_stays_in_tune() {
    let r = plain(driver!(Plugin))
        .set_param(P::HwRes, 1.0)
        .set_param(P::Portamento, 0.3)
        .duration(Duration::from_millis(1600))
        .script(|s| {
            s.note_on(57, 0.8);
            s.wait_ms(300);
            s.note_off(57);
            s.note_on(69, 0.8);
        })
        .run();
    assertions::assert_no_nans(&r);
    let hz = crossing_hz(&r.output[0][r.output[0].len() - 48_000..], SR);
    assert!((hz - 440.0).abs() < 2.0, "measured {hz} Hz");
}

#[test]
fn process_is_allocation_free() {
    assert_no_audio_alloc(|| {
        driver!(Plugin)
            .sample_rate(SR)
            .duration(Duration::from_millis(60))
            .script(|s| {
                for n in 48..72 {
                    s.note_on(n, 0.8);
                }
                s.set_param(P::ArpOn, 1.0);
                s.set_param(P::KeyMode, 0.0);
                s.wait_ms(20);
                s.set_param(P::Chorus, 1.0);
                s.cc(64, 1.0);
                s.wait_ms(20);
                s.cc(64, 0.0);
            })
            .run()
    });
}

#[test]
#[ignore]
fn print_levels() {
    let db = |x: f32| 20.0 * x.max(1e-9).log10();
    for (name, notes, res) in [("1 note", 1, 0.0), ("6 notes", 6, 0.0), ("6 notes res", 6, 1.0)] {
        let r = driver!(Plugin)
            .sample_rate(SR)
            .set_param(P::VcfRes, res)
            .duration(Duration::from_millis(500))
            .script(move |s| {
                for n in 0..notes {
                    s.note_on(48 + 4 * n as u8, 0.8);
                }
            })
            .run();
        let x = &r.output[0][9600..];
        let peak = x.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        println!("{name}: rms {:.1} dBFS, peak {:.1} dBFS", db(rms(x)), db(peak));
    }
}

#[test]
#[ignore]
fn bench_realtime_factor() {
    for os in [0.0, 0.5, 1.0] {
        let start = std::time::Instant::now();
        let r = driver!(Plugin)
            .sample_rate(SR)
            .set_param(P::Oversample, os)
            .set_param(P::Chorus, 1.0 / 3.0)
            .duration(Duration::from_secs(10))
            .script(|s| {
                for n in 0..6u8 {
                    s.note_on(48 + 3 * n, 0.8);
                }
            })
            .run();
        let el = start.elapsed().as_secs_f64();
        println!("os {os}: 10 s audio in {el:.3} s -> {:.0}x realtime ({} frames)", 10.0 / el, r.output[0].len());
    }
}

#[test]
#[ignore]
fn print_dco_cv_transient() {
    let r = plain(driver!(Plugin))
        .set_param(P::KeyMode, 0.0)
        .set_param(P::MonoRetrig, 0.0)
        .set_param(P::Voices, 0.0)
        .duration(Duration::from_millis(400))
        .script(|s| {
            s.note_on(48, 0.8);
            s.wait_ms(200);
            s.note_on(72, 0.8);
        })
        .run();
    let x = &r.output[0];
    let jump = 9600; // 200 ms
    for (a, b) in [(jump - 960, jump), (jump, jump + 48), (jump + 48, jump + 144), (jump + 960, jump + 1920)] {
        let pk = x[a..b].iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        println!("{:>6.2}..{:>6.2} ms peak {pk:.4}", (a as f64 - jump as f64) / 48.0, (b as f64 - jump as f64) / 48.0);
    }
}

/// Every compiled-in preset is well-formed and plays at a sane level.
#[test]
fn bank_presets_play() {
    use truce_params::Params;
    let infos = Youth106Params::default().param_infos();
    assert!(bank::BANK.len() >= 299, "{} presets", bank::BANK.len());
    for preset in bank::BANK {
        for (id, plain) in preset.params {
            let info = infos.iter().find(|i| i.id == *id).unwrap_or_else(|| panic!("{}: id {id}", preset.name));
            let (lo, hi) = (info.range.denormalize(0.0), info.range.denormalize(1.0));
            assert!(*plain >= lo - 1e-9 && *plain <= hi + 1e-9, "{}: {id} = {plain} outside {lo}..{hi}", preset.name);
        }
        let mut d = driver!(Plugin).sample_rate(SR).duration(Duration::from_millis(2500));
        for (id, n) in bank::panel_targets(preset.params, &infos) {
            d = d.set_param(id, n);
        }
        let r = d
            .script(|s| {
                for n in [48, 55, 60, 64] {
                    s.note_on(n, 0.85);
                }
                s.wait_ms(2000);
            })
            .run();
        assertions::assert_no_nans(&r);
        let peak = r.output[0].iter().chain(&r.output[1]).fold(0.0f32, |m, x| m.max(x.abs()));
        println!("{:<14} {:<32} peak {:6.1} dB", preset.category, preset.name, 20.0 * peak.max(1e-9).log10());
        // Factory patches keep their stored VCA LEVEL even when a chord
        // runs a little over 0 dBFS; the patches we author must fit.
        let factory = preset.category.starts_with("Juno-");
        let ceiling = if factory { 2.1 } else { 1.0 };
        assert!(peak > 0.03 && peak < ceiling, "{}/{}: peak {peak}", preset.category, preset.name);
    }
}

#[test]
fn bank_has_every_factory_patch() {
    for (category, count) in [("Juno-106 A", 64), ("Juno-106 B", 64), ("Juno-60 A", 56), ("Juno-60 B", 55)] {
        assert_eq!(bank::BANK.iter().filter(|p| p.category == category).count(), count, "{category}");
    }
}

#[test]
fn bank_names_are_unique_within_a_category() {
    let mut seen = std::collections::HashSet::new();
    for p in bank::BANK {
        assert!(seen.insert((p.category, p.name)), "{}/{}", p.category, p.name);
    }
}

/// A preset load leaves the machine alone: settings, level and tuning.
#[test]
fn presets_leave_the_instrument_settings_alone() {
    use truce_params::Params;
    let infos = Youth106Params::default().param_infos();
    let targets = bank::panel_targets(bank::BANK[0].params, &infos);
    for p in [P::Master, P::Tune, P::Transpose, P::Voices, P::Oversample, P::Velocity, P::NoiseFloor, P::HwRes, P::BendDco, P::MidiMod] {
        assert!(targets.iter().all(|(id, _)| *id != u32::from(p)));
    }
    assert!(targets.iter().any(|(id, _)| *id == u32::from(P::VcfFreq)));
    assert!(targets.iter().any(|(id, _)| *id == u32::from(P::KeyMode)), "key mode belongs to the sound");
}

/// Saving, overwriting, reloading and deleting a user preset.
#[test]
fn user_preset_round_trips() {
    use truce_params::Params;
    let infos = Youth106Params::default().param_infos();
    let root = std::env::temp_dir().join(format!("youth106-presets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = bank::user_store().with_user_root(&root);

    let normalized = |id: u32| if id == u32::from(P::VcfFreq) { 0.25 } else { 0.5 };
    let snapshot = bank::panel_snapshot(&infos, normalized);
    assert!(snapshot.iter().all(|(id, _)| *id != u32::from(P::Master)), "master is not part of a sound");

    let saved = bank::save_user(&store, "Round Trip", &snapshot).unwrap();
    assert_eq!(bank::user_presets(&store).len(), 1);
    bank::save_user(&store, "Round Trip", &snapshot).unwrap();
    assert_eq!(bank::user_presets(&store).len(), 1, "same name overwrites");

    let loaded = store.load(&saved.uuid).unwrap();
    let freq = bank::panel_targets(&loaded.params, &infos).into_iter().find(|(id, _)| *id == u32::from(P::VcfFreq)).unwrap();
    assert!((freq.1 - 0.25).abs() < 1e-6);

    store.delete(&saved.uuid).unwrap();
    assert!(bank::user_presets(&store).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// Projects saved before presets existed hold no persisted name.
#[test]
fn state_without_a_preset_name_loads() {
    use truce_params::Params;
    let params = Youth106Params::default();
    params.load_persist(&[]);
    assert!(params.current_preset.lock().unwrap().is_empty());
}
