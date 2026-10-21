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

