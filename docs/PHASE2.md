# Phase 2 — hardware fidelity analysis

Sources: `juno106_service_manual.pdf` (Roland Service Notes, 1st ed., Jul 1984;
pages cited as p.N), Ultramaster KR-106 docs, community research (links at end).
No code changed yet — this is the findings list and proposed work.

## 1. What the manual confirms (no change needed)

Firmware scales in `firmware.rs` reproduce the official spec (p.1) exactly
(1 VCF octave = 1143 accumulator counts):

| Spec (p.1) | Model | |
|---|---|---|
| VCF ENV MOD ±14 oct | 254·0x3FFF>>8 = 16256 counts = 14.2 oct | ✓ |
| VCF LFO MOD ±3.5 oct | 4048 counts = 3.54 oct | ✓ |
| VCF BENDER ±3.5 oct | 255·255>>4 = 4064 counts = 3.56 oct | ✓ |
| KEY FOLLOW +3/−2 oct | over C1–C6 (MIDI 36–96) at KYBD 10: +3.0 / −2.0 oct | ✓ |
| Cutoff 5 Hz–50 kHz | DAC table 5.7 Hz–52 kHz | ✓ |
| DCO LFO MOD ±400 ct | `dco_lfo_depth` max 4 st | ✓ |
| DCO BENDER ±1200 ct | Bend Range max 12 st | ✓ |
| Decay/Release 1.5 ms–12 s | ROM table + CalcDecay | ✓ |
| LFO rate up to 30 Hz | 29.5 Hz | ✓ |

Per-voice tolerances also match the adjustment procedure (p.18–19):
- PWM 50% within 48–52%, 95% within 93–97% → our `pw_min/max_offset` ±2%.
- VCF freq/width "within ±10 cents" → our trims ±10 counts (≈10.5 ct), ±10 ct/oct.
- Chorus and HPF component networks (p.15: C10/C11, bass-boost R/C values,
  Tr13–Tr16 BBD filters, IC6 summer R70/R71/R72) are the ones the KR-106 modeled.

VCF anchor: the manual's test table (p.18) lists VCF FREQ 6.3 for the 248 Hz
calibration, but the KR-106 takes byte 49 (≈3.9) from the ROM test patch and
confirms it with 93 measured points on a real unit; 6.3 would give a slider range
of 0.5 Hz–9.6 kHz, contradicting the 5 Hz–50 kHz spec. Keep the current table;
the 6.3 looks like a misprint (still worth checking in the ROM disassembly).

## 2. Deviations from the hardware, ranked by audible impact

### A. Chorus (high impact, schematic-backed)
1. **The 106 has no I+II mode.** The jack board (p.15) has only two control
   lines, CHORUS ON/OFF and CHORUS I/II, and one LFO (IC1). I+II is a
   Juno-6/60 feature the KR-106 inherited. → keep as a clearly labelled
   "extra" or remove; never the default.
2. **Chorus I and II have the same delay depth.** IC1a (Schmitt) + IC1b
   (integrator) make a triangle whose amplitude is set only by R7/R6
   (±0.70·Vsat); the I/II switch (Tr2 → JFET Tr1) only changes the integrator
   input divider (R4 680K vs 680K+R3 2.2M). Computed nominal rates:
   I ≈ 0.55 Hz, II ≈ 0.90 Hz (ratio 1.62). With the KR-106's measured
   I = 0.514 Hz, II ≈ 0.835 Hz. The model uses I 2.13 ms / II 1.71 ms depth
   (J6 values, flagged "not re-verified") → II should use 2.13 ms too.
   "II is stronger" comes from the higher rate (Doppler ∝ rate × depth).
3. Juno-60 measurements (jpcima): delay 1.54–5.15 ms, 0.513 Hz, triangle duty
   49.33% (slightly asymmetric). The 106 uses the same IC1 topology → a tiny
   rise/fall asymmetry is plausible; low priority.

### B. DCO phase behaviour (medium, documented)
4. **The DCOs never reset phase.** The 8253 counters and integrators free-run
   (electricdruid; counter reload doesn't restart the count). The model resets
   phase whenever an idle voice is triggered (`Voice::trigger` → `osc.reset()`),
   so every note starts identically. → remove the reset; notes start at
   arbitrary phase like the hardware.
5. **Unison is phase-locked and perfectly in tune.** All DCOs divide one 8 MHz
   crystal; Roland's unison (Poly 1 + Poly 2, confirmed by Roland) has no detune.
   The model's "Unison Drift" defaults to 0.25 (static detune + walk). →
   default 0 with fixed, random per-voice phase offsets; keep drift as an opt-in
   extra.

### C. Control-rate stepping (medium, hardware-math; community divided)
6. **Panel sliders are 7-bit.** VCF FREQ is stored as byte<<7, i.e. 128 steps
   of 128 counts = 0.112 oct ≈ **1.34 semitones per step**; the 0.5 ms CV RC
   doesn't smooth steps that are held for many ticks. A manual sweep on the
   hardware is stepped (some owners describe the 106 resonance as "stepped",
   others don't hear it). Our sliders are continuous. → optional
   "Hardware resolution" switch (7-bit sliders), off by default so DAW
   automation stays smooth.
7. **Pitch modulation is frame-rate and integer.** Divider value = key + LFO +
   bender + portamento + tune, recomputed per 4.2 ms frame (p.8). Portamento
   and bend are therefore 234 Hz staircases; the bender ADC is 8-bit (≈9 ct
   steps at 12 st range). Our glide is per-sample and bend is continuous. →
   tick-rate pitch update + 8-bit bend in the same "hardware resolution" mode.
   (Divider quantization itself is < 1 cent below MIDI ~120: negligible.)

### D. Waveform details (low–medium)
8. **Saw amplitude is only "nearly constant".** Amplitude = DCO CV / frequency
   (C54 charged through R85–87, p.9); DCO CV is per-voice, per-frame through a
   S/H. During fast pitch jumps the amplitude lags for a frame (users report
   clicks on big jumps), and since the pulse is "saw > PWM CV" (50% at 6 V,
   95% at 0.6 V of a 12 Vpp ramp) the pulse width wobbles with it.
9. **PWM CV is one shared DAC channel** (p.8 timeline) → per-voice ENV PWM can't
   exist on a 106; the panel switch is LFO/MAN. Our ENV position is a J60
   extra. With PWM OFF the CV is −0.8 V, holding the comparator high (DC only).
10. Saw reset is a transistor discharge on the clock edge (not instantaneous);
    tiny, but sets the HF content of the 106 saw.

### E. Small parameter mismatches
11. Fine tune is ±50 cents on the hardware (we have ±100).
12. LFO rate low end: spec says 0.1 Hz, ROM table gives 0.036 Hz — keep ROM,
    note the discrepancy.
13. LFO delay: spec 0–3 s; model holdoff 3.3 s + ramp ~1.1 s at max.
14. VCA offset trimmers are adjusted "for minimum thumps" (p.18): real units
    keep a small per-voice CV-feedthrough thump, notably in GATE mode. Not
    modeled; could be an optional per-voice amount.

## 3. Calibration targets for an A/B test harness (from p.18–19)

Render these with `truce_test` and check ratios (absolute volts don't apply):
- Bank 3 (FREQ byte 49, RES 10, KYBD 10, SUS 10), C4 → 248 Hz self-osc;
  C6 → 992 Hz (±10 ct).
- Same patch: VCF out 4.8 Vpp vs VCA out 6 Vpp sine; Bank 6 noise at VCA out
  4 Vpp → noise : self-osc ratio 4 : 6 at the VCA output.
- Bank 5 PWM 50% / PWM 10 → 95% duty.
- Chorus bias: symmetric clipping at 10 Vpp into the BBD (sat-drive check).

## 4. Implementation status

| # | Item | Status |
|---|---|---|
| 1 | Chorus II depth = I depth, II rate 0.835 Hz, I+II labelled "(J60)" extra | done |
| 2 | DCO free-run (no phase reset), fixed random per-voice phase, unison drift default 0 | done |
| 3 | Fine tune ±50 ct | open |
| 4 | "HW Res" setting: 7-bit sliders (PWM /105), per-frame portamento and bend, 8-bit bender | done |
| 5 | Saw height = DCO CV / f through a per-voice S/H (τ ≈ 1 ms, estimated), pulse width follows the ramp height | done |
| 6 | Optional VCA thump, LFO/chorus triangle asymmetry | open |
| 7 | A/B harness with the manual's calibration targets, then recordings | open |
| 8 | BBD hiss (broadband + clock-dependent leakage that pans with the chorus LFO) removed by choice | done |

## Sources

- Roland JUNO-106 Service Notes, 1st ed. (`juno106_service_manual.pdf` in the repo root): p.1 specs, p.8–9 circuit
  description, p.15 jack board, p.18–19 adjustment.
- Electric Druid, "Roland Juno DCOs": https://electricdruid.net/roland-juno-dcos/
- Roland, "Icon Series: JUNO-106" (assign modes, unison):
  https://www.roland.com/uk/blog/roland-icon-series-juno-106-synthesizer/?lang=en-GB
- jpcima/rc-effect-playground #2 (Juno-60 chorus measurements):
  https://github.com/jpcima/rc-effect-playground/issues/2
- SoundForce Chorus 6 (106 BBD driver sampled, 180° L/R): https://sound-force.nl/?page_id=5000
- Mod Wiggler, "Roland Juno Filter Stepping": https://modwiggler.com/forum/viewtopic.php?t=180175
- Vintage Synth Explorer, "Juno-60 vs Juno-106":
  https://forum.vintagesynth.com/viewtopic.php?t=69980&start=15
- Ultramaster KR-106 `docs/vcf_modulation_budget.md` (byte 49 anchor).
