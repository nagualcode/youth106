//! Prints the engine's panel curves as CSV, for `tools/gen_presets.py`:
//! `cargo run --release --example curves > tools/juno106_curves.csv`

use youth106::dsp::{adsr, firmware, lfo};

fn main() {
    println!("pos,vcf_hz,lfo_hz,lfo_delay_ms,dco_lfo_st,vcf_lfo_st,sub,noise,attack_ms,dec_rel_ms");
    for i in 0..=1024 {
        let t = i as f32 / 1024.0;
        println!(
            "{t:.6},{:.3},{:.4},{:.2},{:.4},{:.4},{:.5},{:.5},{:.2},{:.2}",
            firmware::vcf_freq_hz(t),
            lfo::lfo_rate_hz(t),
            lfo::lfo_delay_ms(t),
            firmware::dco_lfo_depth(t),
            firmware::vcf_lfo_semitones(t),
            firmware::dco_sub_level(t),
            firmware::dco_noise_level(t),
            adsr::attack_ms(t),
            adsr::dec_rel_ms(t),
        );
    }
}
