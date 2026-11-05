# Youth106

**A Roland Juno-106 emulation for modern DAWs, by Jardim Sonoro.**

Youth106 is a free, non-commercial study in hardware simulation: a
six-voice Juno-106 rebuilt in Rust with the [truce](https://github.com/truce-audio/truce)
plugin framework. The sound engine follows the 106's actual design: the
voice CPU firmware math, the IR3109 ladder filter, the multiplexed DAC
timing and the MN3009 BBD chorus. The interface is a flat, Ableton-style
panel laid out in the Juno-106's own section order.

![Youth106](screenshots/main.png)

- Formats: **VST3**, **AU**, **CLAP** on macOS (Apple Silicon); **VST3**, **CLAP** on Windows and Linux (x64)
- 299 presets browsed inside the plugin: the 128 Juno-106 and 111 Juno-60 factory patches, plus sounds from records
- Save, overwrite and delete your own presets from the plugin's interface
- Every control is an automatable host parameter, with real units shown in the DAW

## Install

Download the installer for your system from [Releases](../../releases). The
builds are not code-signed, so the system will warn you the first time.

- **macOS** (Apple Silicon; AU, VST3, CLAP): `youth106-*-macos.pkg`. If macOS
  blocks it, right-click the file and choose *Open*.
- **Windows** (x64; VST3, CLAP): the `.exe` installer. In the SmartScreen
  warning choose *More info › Run anyway*.
- **Linux** (x64; VST3, CLAP): unpack the tarball and run `./install.sh`.

Then rescan plug-ins in your DAW: *Jardim Sonoro › Youth106*.

Upgrading from 0.1: the factory patches no longer live in your DAW's preset
list. If you copied them to `~/Library/Audio/Presets/Jardim Sonoro/Youth106/`,
you can delete that folder.

## Using it

Play it from any MIDI track. Pitch bend, mod wheel (CC1) and sustain pedal
(CC64) work as standard. Controls: drag to change, **Shift**-drag for fine
moves, **double-click** to reset, mouse wheel for small steps.

### Top row — the Juno-106 panel

| Section | Controls | What it does |
|---|---|---|
| **LFO** | Rate, Delay, Auto/Man, Sync + division | Global triangle LFO. Delay fades it in after a key press (Auto: only when all keys were up; Man: on every key or mod-wheel move). Sync locks it to the DAW tempo. On its own it makes no sound: raise DCO LFO (vibrato), VCF LFO (filter wobble) or set PWM to LFO. The chorus has its own fixed LFO. |
| **DCO** | LFO, PWM, Sub, Noise, Range 16'/8'/4', PWM source, Pulse, Saw | The oscillator. LFO = vibrato depth. PWM = pulse width (manual, or swept by the LFO). Sub = square one octave down. Noise = white noise. Pulse and Saw switch the two main waveforms. |
| **HPF** | 4-position switch | High-pass: bass boost, flat, 236 Hz, 754 Hz. |
| **VCF** | Freq, Res, Env (+/− polarity), LFO, Kybd | 24 dB/oct resonant low-pass; self-oscillates at full Res. Env/LFO set how much the envelope and LFO move the cutoff; Kybd makes it follow the keyboard. |
| **VCA** | Level, Env/Gate | Output level of the patch (±10 dB). Env shapes volume with the envelope; Gate is on/off with the key. |
| **ENV** | A, D, S, R | One envelope for both the VCF and VCA, with a live shape preview. |
| **CHORUS** | I, II | The 106's stereo BBD chorus: I is gentle, II is faster. Pressing both gives the Juno-60's I+II mode, which the 106 doesn't have. |

A real 106's chorus also adds a hiss that slowly swaps between left and
right (each MN3009 leaks more noise when its clock is slow, and the two BBDs
are swept in antiphase). Youth106 leaves that hiss out; the chorus
modulation, filtering and BBD saturation are modeled as on the hardware.

### Bottom row — performance

| Section | Controls | What it does |
|---|---|---|
| **KEYS** | Mono/Poly 1/Poly 2, Porta, Hold, Transpose | Poly 1: release tails ring out. Poly 2: notes reuse voices, so only the latest note keeps its release. Mono: all six voices on one note (the 106's unison). Porta = glide time. Hold sustains notes (the sustain pedal does the same). |
| **CONTROLLER** | Bend, Bend VCF, Mod LFO, Tune | Pitch-bend range in semitones, how much bend moves the filter, how much the mod wheel adds LFO vibrato, and fine tune. |
| **ARPEGGIO** | On, Up/U&D/Down, 1–3 octaves, Rate, Sync + division | Arpeggiator over held notes (a Juno-6/60 feature, included as an extra). |
| **OUTPUT** | Meter, Master | Output level. |

### Settings (header button)

Voices (6/8/10), Oversampling (1×/2×/4×, default 2×), Velocity (off on the
real 106), Mono Retrigger, Unison Drift (adds detune in Mono; 0 = hardware),
Analog Noise (the noise floor, mains hum and chorus clicks of the original;
0 removes it),
and **HW Res** — hardware resolution: 7-bit panel sliders (the filter
cutoff moves in ~1.3-semitone steps), portamento and bend updated every
4.2 ms, and an 8-bit bender, like the real firmware. Leave it off for
smooth automation.

## Presets

The 299 presets live inside the plugin, not in your DAW's preset list. The bar
at the top of the editor steps through them (`<` `>`); clicking the name opens
the browser, with categories and a search that also covers artists and songs.
Clicking a preset loads it and leaves the browser open, so you can audition
from the keyboard.

**SAVE** stores the panel under the *User* category, as one file per preset
shared by every plugin format; saving under an existing name overwrites it,
and hovering a user preset shows `×` to delete it. The project remembers which
preset is loaded.

A preset is a sound. What a 106 keeps out of its patch memory stays as it is
when one loads: the Settings page, Master, Tune, Transpose and the Bend and Mod
LFO amounts.

- **Juno-106 A / B**: the 128 factory patches.
- **Juno-60 A / B**: 111 of the 112 factory patches, moved onto the 106 engine
  in physical units (Hz, semitones, ms), so they sound like the 60's patches on
  a 106 rather than like the same slider positions. B34 Drawbar Organ is lost:
  its tape data is blank.
- **Classics**: 52 sounds from records, including Tame Impala, Wham!'s *Last
  Christmas*, Daft Punk's *Short Circuit*, Mac DeMarco, Blood Orange,
  BADBADNOTGOOD and Porches. Each comment names the record and the grade:
  *Recipe* follows published settings; *Reconstruction* is our estimate where
  nothing was published.
- **Sound Design**: seven basses and pads from Roland and Attack Magazine
  tutorials.

None of the Classics were compared by ear with the records.
[docs/PRESETS.md](docs/PRESETS.md) has the sources and the limits of each bank.

Factory patches keep their stored VCA Level. Some of them run over 0 dBFS on a
chord, up to about +6 dB, so lower Master if your DAW's meter complains.

## The audio path

```
 MIDI ─► voice assignment (Poly 1 / Poly 2 / Mono, as in the 106's ROM)
          │
          ▼  × 6 voices (or 8/10), firmware tick at ~234 Hz
   ┌──────────────────────────────────────────────────────────────┐
   │ DCO (saw + pulse + sub, one free-running counter; saw height  │
   │      set by the DCO CV) + NOISE                               │
   │   → coupling cap → IR3109 4-pole OTA ladder VCF (BA662 loop)  │
   │   → coupling cap → VCA (measured exponential converter)      │
   │ Envelope, VCF CV and VCA CV are integers written to a         │
   │ multiplexed DAC, each voice at its own time slot, then RC     │
   │ smoothed — exactly as the D7811G voice CPU does.              │
   └──────────────────────────────────────────────────────────────┘
          │ summed at 2× sample rate, half-band decimated
          ▼
   HPF (boost / flat / 236 / 754 Hz) → analog noise floor + mains ripple
          ▼
   VCA LEVEL (µPC1252H2, ±10 dB) → output pole
          ▼
   MN3009 BBD chorus, two lines in antiphase → stereo (dry + wet per side)
          ▼
   Master
```

Source map: `src/dsp/` — `voice.rs` (voice + DAC timing), `osc.rs` (DCO),
`vcf.rs` (filter), `adsr.rs` (envelope), `firmware.rs` (CPU math, tables),
`lfo.rs`, `filters.rs` (HPF, decimator), `chorus.rs`, `noise.rs`, `arp.rs`,
`mod.rs` (voice assignment and the global chain). UI in `src/ui/`. Presets:
`bank/` (generated by `tools/gen_presets.py`, compiled in by `build.rs`) and
`src/bank.rs`.

## How it was made

1. **Reference port.** The engine starts from the Juno-106 model of the
   open-source [Ultramaster KR-106](https://github.com/kayrockscreenprinting/ultramaster_kr106)
   (GPL-3.0), which is built on ROM disassembly of the 106's voice CPU and on
   measurements of real units: the envelope's integer math and ROM tables,
   the firmware's cutoff routine, a VCF cutoff table measured at every
   DAC code, the VCA curve, the LFO speed table, and ngspice-verified chorus
   filters. Only the Juno-106 path was ported. Its own keyboard and MIDI
   handling were left out in favour of standard DAW MIDI.
2. **Checking against the service manual.** The Roland *Juno-106 Service
   Notes* (1984, `juno106_service_manual.pdf`) were read section by section: specifications, circuit
   descriptions, the module and jack board schematics and the adjustment
   procedure. The firmware scales match the official specs (VCF env ±14 oct,
   LFO and bend ±3.5 oct, key follow +3/−2 oct, cutoff 5 Hz–50 kHz,
   DCO LFO ±400 cents), and the per-voice tolerances match the calibration
   limits.
3. **Community knowledge.** Electric Druid's DCO analysis, Roland's own
   notes on assign modes, Juno chorus measurements and forum discussions
   about stepping, unison and calibration were used to cross-check behaviour.
4. **Corrections from that research** (beyond the reference):
   - **Chorus:** the 106's jack board has one chorus LFO and just two control
     lines. Working through the Schmitt-trigger/integrator circuit shows I and
     II differ only in rate (0.514 / 0.835 Hz), at the **same** delay depth.
     I+II is a Juno-60 mode, kept only as a labelled extra.
   - **DCO phase:** the 8253 counters free-run, so notes never restart the
     oscillator phase, and unison voices sit at fixed, arbitrary phase
     offsets instead of detuning (drift now defaults to 0).
   - **DCO CV coupling:** the saw's height is the DCO CV divided by
     frequency, and the pulse is "ramp above the PWM CV". The CV is a
     per-voice sample-and-hold, so after pitch jumps the ramp height and the
     pulse width briefly move, as on the hardware.
   - **Hardware resolution mode:** the firmware's 7-bit sliders and 4.2 ms
     pitch frames, as an option.
   - **No chorus hiss:** the BBD hiss is deliberately not modeled.
   - Output calibrated for DAW gain staging (a full chord fits under 0 dBFS
     at Master 0 dB), and factory PWM values scaled to the 106's 0–105 range.

   `docs/PHASE2.md` lists every finding, the page references and what
   remains open.

## Build from source

```sh
cargo install cargo-truce
cargo truce install        # build + install VST3/AU/CLAP
cargo truce run            # standalone
cargo test --release       # DSP, plugin and preset tests
cargo truce validate       # auval, pluginval, clap-validator
```

Validation: AU passes auval; VST3 passes pluginval at strictness 10. The
CLAP build fails clap-validator's three `state-reproducibility` tests;
truce's own example synth fails the same three, so the problem is in the
framework's CLAP wrapper, not this plugin.

Pushing a tag `vX.Y.Z` makes [GitHub Actions](.github/workflows/release.yml)
build all three platforms and publish them as a release. Pull requests and
manual runs build the same installers and keep them as workflow artifacts.

## Credits and license

- DSP derived from the Ultramaster KR-106 by Karl LaRocca et al. (GPL-3.0);
  ROM disassembly by Gordon Pearce.
- Plugin framework: truce (MIT/Apache-2.0).
- Preset research: Reverb Machine; Roland Articles; Attack Magazine; Synthmania.
- Juno-106, Juno-60 and factory patch data © Roland Corporation. Artist and
  song names are used only to describe the style of a preset. Youth106 is an
  independent, non-commercial project, not affiliated with Roland or with any
  of the artists mentioned.

Youth106 is licensed under **GPL-3.0-or-later**.
