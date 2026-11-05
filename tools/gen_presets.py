#!/usr/bin/env python3
"""Generate bank/ (truce authored-TOML presets, numeric param ids).

Usage, from the repo root:
  cargo run --release --example curves > tools/juno106_curves.csv
  python3 tools/gen_presets.py <ultramaster_kr106>/Source/KR106_Presets_JUCE.h

bank/ is compiled into the plugin by build.rs. A preset names only the
parameters it sets; the rest load at their defaults (see src/bank.rs).

- Juno-106 A/B: the 128 factory patches, decoded from Roland's SysEx.
- Juno-60 A/B: the 112 factory patches, from a Juno-60 tape dump, moved
  onto the 106 engine in physical units (see convert_j60).
- Classics: sounds from records, graded in each comment. "Recipe" follows
  settings published by the cited source; "Reconstruction" means no
  settings were published and the values are our own estimate.
- Sound Design: published programming recipes, not tied to one record.

The KR-106 header holds both banks as raw 7-bit values in KR-106 EParams
order (indices below).
"""
import csv
import math
import re
import shutil
import sys
import uuid
from pathlib import Path

OUT = Path("bank")
NS = uuid.UUID("6f1d5c2a-1060-4a10-9b06-000000106106")

IDS = {
    "arp_on": 1, "arp_mode": 2, "arp_range": 3, "arp_rate": 4, "arp_sync": 5, "arp_div": 6,
    "lfo_rate": 10, "lfo_delay": 11, "lfo_mode": 12, "lfo_sync": 13, "lfo_div": 14,
    "dco_range": 20, "dco_lfo": 21, "dco_pwm": 22, "pwm_mode": 23, "dco_pulse": 24, "dco_saw": 25,
    "dco_sub": 26, "dco_noise": 27, "hpf": 30,
    "vcf_freq": 40, "vcf_res": 41, "vcf_env_pol": 42, "vcf_env": 43, "vcf_lfo": 44, "vcf_kbd": 45,
    "vca_mode": 50, "vca_level": 51, "env_a": 60, "env_d": 61, "env_s": 62, "env_r": 63,
    "chorus": 70, "key_mode": 83, "portamento": 84, "hold": 85,
}

# Switch positions, as in src/lib.rs.
PWM_LFO, PWM_MAN, PWM_ENV = 0, 1, 2
BOOST, FLAT, HZ236, HZ754 = 0, 1, 2, 3
ENV, GATE = 0, 1
OFF, I, II, I_II = 0, 1, 2, 3
MONO, POLY1, POLY2 = 0, 1, 2
DIV_1_16 = 9

R = lambda x: round(x, 4)


# --- Curves ---------------------------------------------------------------

def load_curves() -> list[dict]:
    with open("tools/juno106_curves.csv") as f:
        return [{k: float(v) for k, v in row.items()} for row in csv.DictReader(f)]


CURVES = load_curves()


def slider_for(column: str, target: float) -> float:
    """Position on the 106 curve closest to `target` (some are stepped)."""
    if target <= 0.0:
        return 0.0
    best = min(CURVES, key=lambda r: abs(math.log(max(r[column], 1e-6) / target)))
    return R(best["pos"])


def lerp(table: list[float], t: float) -> float:
    i = min(int(t * 10), 9)
    return table[i] + (t * 10 - i) * (table[i + 1] - table[i])


# Juno-60 panel curves, transcribed from the formulas documented in
# Ultramaster KR-106 (GPL-3.0): KR106VcfFreqJ6.h, KR106LFO.h, KR106Voice.h
# and KR106ADSR.h. KR-106 flags its 60 envelope, sub and DCO LFO curves as
# Juno-6 measurements standing in for the 60, so these are close, not exact.

def j60_vcf_hz(s):
    f = 28.0 * math.exp(7.262 * s)
    return f if f <= 20000.0 else 20000.0 + 20000.0 * math.tanh((f - 20000.0) / 20000.0)


def j60_lfo_hz(s):
    vr2, r24, r25, r32, c6, vsat, beta = 50000.0, 4700.0, 330.0, 150000.0, 1e-7, 13.5, 33000.0 / 47000.0
    alpha = (10.0 ** (2.0 * s) - 1.0) / 99.0
    r_up = (1.0 - alpha) * vr2 + r24
    r_dn = alpha * vr2 + r25
    vw = vsat / (r_up * (1.0 / r_up + 1.0 / r_dn + 1.0 / r32))
    return vw / (4.0 * vsat * beta * c6 * r32)


def j60_dco_lfo_st(s):
    return lerp([0.0, 0.041, 0.132, 0.198, 0.289, 0.405, 0.487, 0.940, 1.440, 2.053, 2.526], s)


def j60_vcf_lfo_st(s):
    return 36.0 * s**1.2


def j60_env_oct(s):
    nat = lerp([0.0, 0.227, 1.114, 1.984, 2.677, 3.510, 4.268, 5.163, 6.064, 6.925, 7.345], s)
    return nat / math.log(2.0)


def j60_sub(s):
    return lerp([0.00285, 0.00285, 0.02512, 0.05820, 0.09661, 0.15139, 0.20654, 0.44158, 0.66834, 0.88106, 1.0], s)


def j60_noise(s):
    if s <= 0.0:
        return 0.0
    d = s - 0.1120
    return 1.1260 * (math.sqrt(d * d + 0.0227 * 0.0227) + d) * 0.5


def j60_attack_ms(s):
    taus = [0.000558, 0.001674, 0.008762, 0.029468, 0.064015, 0.120998, 0.238481, 0.495993, 0.607950, 1.392486, 1.674332]
    x = s * 10.0
    i = min(int(x), 9)
    return 1791.8 * math.exp(math.log(taus[i]) + (x - i) * (math.log(taus[i + 1]) - math.log(taus[i])))


def j60_dec_rel_ms(s):
    return 2397.9 * 0.003577 * math.exp(12.9460 * s - 5.0638 * s * s)


# --- Factory banks --------------------------------------------------------

def inverse_taper(t_post: float) -> float:
    """DCO LFO patch bytes are post analog taper; find the slider."""
    if t_post <= 0.0:
        return 0.0
    if t_post >= 1.0:
        return 1.0
    a = 5.0 * t_post
    b = 1.0 - 5.0 * t_post
    c = -t_post
    return (-b + math.sqrt(b * b - 4.0 * a * c)) / (2.0 * a)


def switches(v: list[int]) -> dict:
    return {
        "dco_range": v[29],
        "pwm_mode": v[33],
        "dco_pulse": bool(v[23]),
        "dco_saw": bool(v[24]),
        "vcf_env_pol": v[34],
        "vca_mode": v[35],
        "chorus": (1 if v[27] else 0) | (2 if v[28] else 0),
    }


def convert_j106(v: list[int]) -> dict:
    """Bytes are 106 panel values: /127, except PWM, limited to 105 by R5/R6."""
    s = lambda i: R(v[i] / 127.0)
    return {
        **switches(v),
        "lfo_rate": s(3),
        "lfo_delay": s(4),
        "dco_lfo": R(inverse_taper(v[5] / 127.0)),
        "dco_pwm": R(min(v[6], 105) / 105.0),
        "dco_sub": s(7) if v[25] else 0.0,
        "dco_noise": s(8),
        "hpf": v[9],
        "vcf_freq": s(10),
        "vcf_res": s(11),
        "vcf_env": s(12),
        "vcf_lfo": s(13),
        "vcf_kbd": s(14),
        "vca_level": s(15),
        "env_a": s(16),
        "env_d": s(17),
        "env_s": s(18),
        "env_r": s(19),
    }


def convert_j60(v: list[int]) -> dict:
    """A 60 slider and a 106 slider at one position do different things, so
    each is moved to the position giving the same Hz, semitones or ms.

    Same on both machines, copied as is: resonance, key follow, sustain,
    VCA level, PWM width (the 60's slider reaches the same width at 127).
    HPF: the 60 has no boost; its flat position is the 106's Flat, and its
    122 Hz position has no 106 equivalent, so it also lands on Flat. The
    106 DCO LFO byte taper does not apply: 60 bytes are slider positions.
    """
    s = lambda i: v[i] / 127.0
    return {
        **switches(v),
        "lfo_rate": slider_for("lfo_hz", j60_lfo_hz(s(3))),
        "lfo_delay": R(s(4)),
        "dco_lfo": slider_for("dco_lfo_st", j60_dco_lfo_st(s(5))),
        "dco_pwm": R(s(6)),
        "dco_sub": slider_for("sub", j60_sub(s(7))) if v[25] else 0.0,
        "dco_noise": slider_for("noise", j60_noise(s(8))),
        "hpf": {0: FLAT, 1: FLAT, 2: HZ236, 3: HZ754}[v[9]],
        "vcf_freq": slider_for("vcf_hz", j60_vcf_hz(s(10))),
        "vcf_res": R(s(11)),
        "vcf_env": R(min(j60_env_oct(s(12)) / 14.0, 1.0)),  # 106: 14 oct at full
        "vcf_lfo": slider_for("vcf_lfo_st", j60_vcf_lfo_st(s(13))),
        "vcf_kbd": R(s(14)),
        "vca_level": R(s(15)),
        "env_a": slider_for("attack_ms", j60_attack_ms(s(16))),
        "env_d": slider_for("dec_rel_ms", j60_dec_rel_ms(s(17))),
        "env_s": R(s(18)),
        "env_r": slider_for("dec_rel_ms", j60_dec_rel_ms(s(19))),
    }


# Remarks on factory patches, shown in the browser and found by search.
J60_NOTES = {
    "A18 Phase Brass": "Phasing from PWM and Chorus I",
    "A34 Clavichord 2": "Wah from the VCF LFO",
    "A55 Popcorn": "Inspired by Hot Butter's Popcorn",
    "A58 PWM Chorus": "Close to the synth line of Sweet Dreams (Eurythmics) (Synthmania)",
    "A61 Synth Organ": "Rob Hyman (Time After Time): \"Number 61 on the JUNO\" - see Classics",
    "B11 Low Strings": "Reminded Synthmania of Deep Honey (Goldfrapp)",
    "B24 Honky Tonk": "Think Bette Davis Eyes (Kim Carnes) (Synthmania)",
    "B28 Pluck II": "Reminiscent of Lucky Man (ELP) (Synthmania)",
}
J106_NOTES = {
    "A14 Flutes": "A la Strawberry Fields Forever (Synthmania)",
    "A15 Moving Strings": "Chorus II",
    "A21 Organ I": "Leslie effect made with PWM, chorus off",
    "A23 Combo Organ": "House of the Rising Sun territory (Synthmania)",
    "A64 Snare Drum": "Reminded Synthmania of the opening of Decades (Joy Division)",
    "B35 Tomita": "After a Tomita synth sound",
    "B38 Bass Pluck": "Heard on many 80s records",
    "B46 Glassy Wah": "Reminds of Wendy Carlos",
}


# --- Classics -------------------------------------------------------------
#
# Reverb Machine programs its recipes on TAL U-NO-LX, with faders marked
# 0-10 (t() below). The fader values are as published. Where an article
# leaves a value to a screenshot, the patch keeps PANEL or a plain musical
# value; docs/PRESETS.md says which sounds are only partly specified.

PANEL = dict(
    lfo_rate=0.3, dco_range=1, dco_lfo=0.0, dco_pwm=0.0, pwm_mode=PWM_MAN, dco_pulse=False, dco_saw=True,
    dco_sub=0.0, dco_noise=0.0, hpf=FLAT, vcf_freq=1.0, vcf_res=0.0, vcf_env_pol=0, vcf_env=0.0, vcf_lfo=0.0,
    vcf_kbd=0.0, vca_mode=ENV, vca_level=0.5, env_a=0.0, env_d=0.5, env_s=0.8, env_r=0.3, chorus=OFF,
)


def t(x: float) -> float:
    return R(x / 10.0)


def hpf(fader: float) -> int:
    """The 106 HPF is a switch: nearest position for a 0-10 fader."""
    return FLAT if fader < 2 else HZ236 if fader < 6 else HZ754


def hz(rate: float) -> float:
    return slider_for("lfo_hz", rate)


def delay_ms(ms: float) -> float:
    return slider_for("lfo_delay_ms", ms)


def patch(**kw) -> dict:
    return {**PANEL, **kw}


RM = "https://reverbmachine.com"
TAME = "Tame Impala"
CLASSICS = [
    # name, comment, params; `None` params marks a factory-derived patch below.
    ("Nangs Wobble", f"{TAME}, Currents (2015). Juno-106 recipe, Reverb Machine; LFO synced 1/16",
     patch(dco_sub=t(6), vcf_freq=t(5), vcf_res=t(1), vcf_lfo=t(2), lfo_sync=True, lfo_div=DIV_1_16, chorus=II)),
    ("Nangs Lead Low", f"{TAME}, Currents. Recipe, Reverb Machine: saw + LFO pulse, vibrato; levels estimated",
     patch(dco_pulse=True, dco_pwm=t(5), pwm_mode=PWM_LFO, dco_lfo=t(2), lfo_rate=t(5), vcf_freq=t(8), env_s=1.0)),
    ("Nangs Lead High", f"{TAME}, Currents. Recipe, Reverb Machine: an octave up, airy, high HPF; levels estimated",
     patch(dco_range=2, dco_pulse=True, dco_pwm=t(5), pwm_mode=PWM_LFO, lfo_rate=t(5), hpf=HZ754, vcf_freq=t(8), env_s=1.0)),
    ("Reality in Motion Brass", f"{TAME}, Currents. Recipe, Reverb Machine",
     patch(dco_pulse=True, dco_pwm=t(3), dco_sub=t(2.5), hpf=hpf(4), vcf_freq=0.0, vcf_env=t(10), env_a=t(5), env_d=t(8),
           env_s=t(3), env_r=t(5), dco_lfo=t(2), lfo_rate=t(3), chorus=II)),
    ("Reality in Motion Thin Pad", f"{TAME}, Currents, bridge. Recipe, Reverb Machine",
     patch(vcf_freq=t(1), vcf_res=t(4), vcf_env=t(5), env_r=t(6), hpf=hpf(4), chorus=II)),
    ("Reality in Motion Vibrato", f"{TAME}, Currents. Recipe, Reverb Machine: CS-80 style brass on a Juno",
     patch(dco_sub=1.0, dco_lfo=t(2), vcf_freq=0.0, vcf_res=t(3), vcf_env=t(7), env_a=t(7), env_r=t(5), chorus=II)),
    ("Yes I'm Changing Pad", f"{TAME}, Currents. Recipe, Reverb Machine; ENV and KYBD estimated",
     patch(dco_sub=t(2), hpf=hpf(2), vcf_freq=t(1.5), vcf_res=t(2), vcf_env=t(2.5), vcf_kbd=t(5), env_r=t(4), env_s=1.0, vca_level=0.8)),
    ("Yes I'm Changing Brass", f"{TAME}, Currents. Recipe, Reverb Machine",
     patch(dco_sub=t(2), hpf=hpf(2), vcf_freq=t(5), vcf_env=t(5), vcf_res=t(4), env_a=t(3), env_d=t(7.5), env_s=0.0, env_r=t(4))),
    ("Mind Mischief", f"{TAME}, Lonerism (2012). Recipe, Reverb Machine; the record adds a JC-120 and a Small Stone phaser",
     patch(dco_saw=False, dco_pulse=True, dco_pwm=t(5), env_s=1.0)),
    ("Nothing That Has Happened", f"{TAME}, Lonerism. Juno-106 recipe, Reverb Machine; amounts estimated",
     patch(dco_saw=False, dco_pulse=True, dco_pwm=t(2), pwm_mode=PWM_LFO, env_d=t(6), env_s=0.0, dco_lfo=t(2.5), hpf=hpf(5), vcf_freq=t(6))),
    ("Keep On Lying Keys", f"{TAME}, Lonerism. Recipe, Reverb Machine: pulse keys; amounts estimated",
     patch(dco_saw=False, dco_pulse=True, vcf_freq=t(7), env_d=t(4), env_s=0.0)),
    ("Cause I'm a Man Pad", f"{TAME}, Currents. Juno-106 recipe, Reverb Machine; ride VCF FREQ",
     patch(dco_saw=False, dco_pulse=True, vcf_freq=t(2), vcf_res=t(3), vcf_env=t(4), env_a=t(4), env_d=t(8), env_s=t(9), chorus=II)),
    ("Cause I'm a Man Lead", f"{TAME}, Currents. Recipe, Reverb Machine: overlap notes to glide",
     patch(dco_sub=t(3), vcf_freq=t(5), vcf_res=t(4), key_mode=POLY1, portamento=t(6))),
    ("Love Paranoia Brass", f"{TAME}, Currents. Juno-106 recipe, Reverb Machine; recorded in mono",
     patch(vcf_freq=t(3.9), vcf_res=t(6.6), vcf_env=t(2.2), vcf_kbd=t(5), vca_mode=GATE, env_a=t(3.4), env_d=t(4.2), env_s=1.0, env_r=0.0, chorus=II)),
    ("Love Paranoia Lead 1", f"{TAME}, Currents. Recipe, Reverb Machine",
     patch(hpf=hpf(5), vcf_freq=t(5), env_d=t(7), env_s=0.0, env_r=t(7), vca_level=0.8, chorus=I)),
    ("Love Paranoia Lead 2", f"{TAME}, Currents. Recipe, Reverb Machine",
     patch(hpf=hpf(6.5), vcf_freq=t(6.5), vcf_res=t(4.5), env_s=1.0, chorus=II)),
    ("She Just Won't Believe Me", f"{TAME}. Reverb Machine: all oscillators, filters open, chorus; push VCA LEVEL to overdrive",
     patch(dco_pulse=True, dco_sub=1.0, vca_level=0.2, env_s=1.0, chorus=I)),

    ("Last Christmas Strings", "Wham! (1984). All parts on a Juno-60. Recipe, Reverb Machine: A13 Strings 3, sub off, shorter release, more VCF env",
     ("A13 Strings 3", dict(dco_sub=0.0, env_r=0.3, vcf_env=0.1))),
    ("Last Christmas Strings 2", "Wham! (1984). Recipe, Reverb Machine: second string layer, Chorus I, softer attack and longer tail",
     ("A13 Strings 3", dict(dco_sub=0.0, env_r=0.45, vcf_env=0.1, env_a=0.2, chorus=I))),
    ("Last Christmas Outro", "Wham! (1984). Reverb Machine: Juno-60 A36 Pizzicato 2, no edits",
     ("A36 Pizzicato 2", dict())),
    ("Last Christmas Bell", "Wham! (1984), the melody. Recipe, Reverb Machine: A15 Organ 2, VCA to ENV, sustain up, release halfway",
     ("A15 Organ 2", dict(vca_mode=ENV, env_s=1.0, env_r=0.5, vcf_env=0.1))),
    ("Last Christmas Organ", "Wham! (1984). Recipe, Reverb Machine: A14 Organ 1, VCA to ENV, no VCF env, Chorus II",
     ("A14 Organ 1", dict(vca_mode=ENV, vcf_env=0.0, chorus=II))),

    ("Chamber of Reflection", "Mac DeMarco, Salad Days. JX-3P on record; Juno recipe, Reverb Machine",
     patch(dco_saw=False, dco_pulse=True, dco_sub=t(5), hpf=hpf(5), vcf_freq=t(6), vcf_res=t(4), vcf_env=t(2), env_d=t(7), env_s=0.0,
           env_r=t(5.5), dco_lfo=t(1.3), lfo_rate=hz(1.24), lfo_delay=delay_ms(230), chorus=I)),
    ("On the Level", "Mac DeMarco. Recipe, Reverb Machine",
     patch(dco_sub=1.0, vcf_freq=t(5), vcf_res=t(3), vcf_env=t(3), env_a=t(2), env_d=t(7), env_s=0.0, env_r=t(6), chorus=I)),
    ("For the First Time Lead", "Mac DeMarco. Recipe, Reverb Machine: On the Level + square, filters to 7, longer release",
     patch(dco_sub=1.0, dco_pulse=True, hpf=hpf(7), vcf_freq=t(7), vcf_res=t(3), vcf_env=t(3), env_a=t(2), env_d=t(7), env_s=0.0, env_r=t(8), chorus=I)),
    ("For the First Time Keys", "Mac DeMarco. Recipe, Reverb Machine",
     patch(hpf=hpf(3), vcf_freq=t(3), vcf_res=t(2), vcf_kbd=t(5), vcf_env=t(3), env_d=t(5), env_s=0.0, env_r=t(4))),
    ("Another One", "Mac DeMarco. Recipe, Reverb Machine: filter wobble at 0.93 Hz",
     patch(dco_pulse=True, dco_sub=1.0, hpf=hpf(5), vcf_env=t(3), vcf_freq=t(5), vcf_lfo=t(5), lfo_rate=hz(0.93))),
    ("Watching Him Fade Away", "Mac DeMarco. Recipe, Reverb Machine",
     patch(hpf=hpf(3.5), vcf_freq=t(1.3), vcf_res=t(2), vcf_env=t(4), vcf_lfo=t(5), vcf_kbd=t(5), lfo_rate=hz(1.32),
           env_a=t(1), env_d=t(6), env_s=t(2.5), env_r=0.0)),
    ("One More Love Song", "Mac DeMarco, This Old Dog (2017). Recipe, Reverb Machine; original synth unknown",
     patch(dco_pulse=True, dco_sub=1.0, vcf_freq=t(3), vcf_res=t(2), vcf_env=t(2.5), vcf_lfo=t(5), lfo_rate=hz(0.93))),

    ("Even the Shadow", "Porches. Almost certainly a Juno-106. Recipe, Reverb Machine",
     patch(hpf=hpf(7.5), vcf_freq=t(0.2), vcf_res=t(2), vcf_env=t(6), vcf_kbd=t(5), env_a=t(4), env_d=t(6), env_s=t(5), env_r=t(5), chorus=II)),
    ("Underwater Arp", "Porches. May be a Juno-106. Recipe, Reverb Machine",
     patch(dco_saw=False, dco_pulse=True, dco_noise=t(4), vcf_freq=t(1.5), vcf_env=t(5), vcf_lfo=t(5), env_a=t(3), env_d=t(4), env_s=0.0,
           env_r=t(4), lfo_rate=t(4), chorus=I)),
    ("Underwater Pad", "Porches. Recipe, Reverb Machine",
     patch(dco_pulse=True, dco_pwm=t(8), hpf=hpf(10), vcf_freq=t(2), vcf_env=t(4), env_a=t(0.5), env_d=t(5), env_s=t(5))),

    ("Structure No.3 Horn", "BADBADNOTGOOD, IV (2016). Recipe, Reverb Machine; ENV amount estimated",
     patch(dco_sub=t(2), vcf_freq=0.0, vcf_env=t(5), env_a=t(3), env_d=t(5), env_s=0.0, env_r=t(5), chorus=I)),
    ("Speaking Gently", "BADBADNOTGOOD, IV. Recipe, Reverb Machine",
     patch(dco_pulse=True, dco_sub=t(4), pwm_mode=PWM_LFO, dco_pwm=t(7), hpf=hpf(6), env_a=t(1), env_d=t(6), env_r=t(5), dco_lfo=t(1), lfo_rate=t(3), chorus=I)),
    ("And That, Too", "BADBADNOTGOOD, IV. Recipe, Reverb Machine",
     patch(dco_saw=False, dco_pulse=True, dco_pwm=t(9), vcf_freq=t(6), vcf_res=t(5), vcf_lfo=t(3.5), lfo_rate=t(2), env_a=t(4), env_r=t(4), chorus=I)),
    ("Lavender Bass", "BADBADNOTGOOD, IV. Recipe, Reverb Machine",
     patch(dco_sub=1.0, vcf_freq=0.0, vcf_env=t(7), vcf_res=t(3), vca_mode=GATE, env_r=t(7), chorus=I)),

    ("Hex Girlfriend Keys", "Neon Indian, Era Extrana. Recipe, Reverb Machine",
     patch(dco_saw=False, dco_pulse=True, dco_pwm=t(9), dco_sub=t(1.5), vcf_freq=t(5), env_s=1.0, chorus=II)),

    ("Short Circuit High Stabs", "Daft Punk, Discovery (2001). Most likely a Juno-106. Recipe, Reverb Machine",
     patch(dco_sub=t(7), lfo_rate=hz(7.24), dco_lfo=t(6.5), lfo_delay=delay_ms(20), env_d=t(3), env_s=0.0, vcf_freq=t(6), vcf_env=t(3))),
    ("Short Circuit Low Stabs", "Daft Punk, Discovery. Recipe, Reverb Machine: high resonance, fast envelope; amounts estimated",
     patch(vcf_freq=t(2.5), vcf_res=t(7), vcf_env=t(4), env_a=0.0, env_d=t(3), env_s=0.0, env_r=t(2))),
    ("Short Circuit Swell", "Daft Punk, Discovery. Recipe, Reverb Machine",
     patch(vcf_freq=t(3.5), vcf_res=t(3.8), vcf_env=t(5), env_a=t(7.2))),
    ("Short Circuit Bass", "Daft Punk, Discovery. Recipe, Reverb Machine",
     patch(dco_sub=t(5.5), vcf_res=t(6.5), vcf_freq=t(3), vcf_env=t(2), env_d=t(5), env_s=t(3), dco_noise=t(3.5))),
    ("Short Circuit Outro", "Daft Punk, Discovery. Recipe, Reverb Machine: slow, wide vibrato",
     patch(dco_lfo=t(1.74), lfo_rate=hz(2.7), env_s=1.0, vcf_freq=t(7), chorus=I)),

    ("Not Good Enough", "Blood Orange, Cupid Deluxe (2013). Recipe, Reverb Machine: muted saw",
     patch(vcf_freq=t(5), env_r=t(5), env_s=1.0, chorus=II)),
    ("Dagenham Dream", "Blood Orange, Negro Swan (2018). Recipe, Reverb Machine; on record through tape wobble",
     patch(hpf=hpf(4), vcf_freq=t(4), env_r=t(5), env_s=1.0, vca_level=1.0, chorus=II)),
    ("Best To You", "Blood Orange. Recipe, Reverb Machine",
     patch(dco_sub=t(2), vcf_freq=t(5), vcf_env=t(3), vca_mode=GATE, env_d=t(3), env_s=0.0, vcf_kbd=t(5), chorus=II)),
    ("Jewelry", "Blood Orange. Recipe, Reverb Machine",
     patch(dco_sub=t(4), vcf_freq=t(4), vcf_env=t(4), env_a=t(6), env_d=t(9), env_s=t(6), env_r=t(7))),
    ("Saint", "Blood Orange. Recipe, Reverb Machine; the record adds 1/8 delay",
     patch(dco_saw=False, dco_pulse=True, dco_pwm=t(5), pwm_mode=PWM_LFO, lfo_rate=t(2), env_a=t(2), env_d=t(8), env_s=0.0, env_r=t(5), chorus=I)),
    ("Charcoal Baby", "Blood Orange, Negro Swan. Recipe, Reverb Machine: the Juno's polyphonic portamento",
     patch(vcf_freq=t(5), env_r=t(5), env_s=1.0, chorus=II, key_mode=POLY1, portamento=t(5))),

    ("Time After Time Pad", "Cyndi Lauper (1983). Rob Hyman: \"Number 61 on the JUNO\". Reconstruction: Juno-60 A61; the bank is not documented",
     ("A61 Synth Organ", dict())),
    ("Take On Me Riff", "a-ha (1985). Juno-60 riff, doubled by another synth on record. Reconstruction: mono, instant attack, full sustain",
     patch(dco_pulse=True, vcf_freq=t(8.5), env_a=0.0, env_s=1.0, env_r=t(4), key_mode=MONO, vca_level=0.4, chorus=I)),
    ("Can You Feel It Bass", "Mr. Fingers (1986). Roland: \"just a JUNO-60, alongside a TR-909\". Reconstruction, no settings published",
     patch(dco_sub=t(6), vcf_freq=t(3), vcf_res=t(2), vcf_env=t(3), vca_mode=ENV, env_d=t(5), env_s=t(3), env_r=t(2), key_mode=POLY2)),
    ("Girls Just Want Fun Solo", "Cyndi Lauper (1983). Reconstruction: steel-drum Juno-60 pad on the arpeggiator; raise BEND VCF for the filter moves",
     patch(dco_saw=False, dco_pulse=True, vcf_freq=t(4), vcf_res=t(4), vcf_env=t(4), env_d=t(4), env_s=0.0, env_r=t(3), arp_on=True, chorus=I)),
]

SOUND_DESIGN = [
    ("Deep House Bass", "Attack Magazine, Make a House Bass with the Juno-60; one envelope stands in for its two",
     patch(dco_pulse=True, dco_sub=0.45, dco_pwm=0.75, pwm_mode=PWM_ENV, vcf_freq=t(5), vcf_res=t(1.5), vcf_kbd=t(5), vcf_env=0.75,
           env_a=0.0, env_d=0.45, env_s=0.25, env_r=t(1.5), chorus=I)),
    ("Synthwave Pluck Bass", "Roland, Creating JUNO Basses: max saw, no square, sub ~65%, plucked filter, chorus off",
     patch(dco_sub=0.65, vcf_freq=0.2, vcf_env=0.4, env_a=0.0, env_d=0.28, env_s=0.0, env_r=0.12)),
    ("Trap Bass", "Roland, Creating JUNO Basses: 16' square, Mono, low VCF; add distortion after it",
     patch(dco_range=0, dco_saw=False, dco_pulse=True, vcf_freq=0.3, vcf_env=R(35 / 255), env_d=R(168 / 255), env_s=0.0, env_r=R(120 / 255), key_mode=MONO, vca_level=0.5)),
    ("Acid Bass", "Roland, Creating JUNO Basses: 8' square, Freq 23/255, Res 204/255, Mono, porta ~20%; add drive and delay",
     patch(dco_saw=False, dco_pulse=True, vcf_freq=R(23 / 255), vcf_res=R(204 / 255), vcf_env=0.5, env_a=0.0, env_d=0.3, env_s=0.0,
           env_r=R(5 / 255), key_mode=MONO, portamento=0.2, vca_level=0.4)),
    ("Original Reese", "Roland, Creating JUNO Basses: full pulse, LFO PWM above 200/255, Freq ~130/255, chorus on; sync the LFO",
     patch(dco_saw=False, dco_pulse=True, pwm_mode=PWM_LFO, dco_pwm=0.8, vcf_freq=R(130 / 255), env_a=0.0, env_d=0.0, env_s=1.0, env_r=0.1,
           lfo_rate=t(3), lfo_delay=0.0, chorus=I)),
    ("Tech House Bass", "Roland, Creating JUNO Basses: 16' square, sub 54/255, Freq ~80, Env 45; add 1/8D delay",
     patch(dco_range=0, dco_saw=False, dco_pulse=True, dco_sub=R(54 / 255), vcf_freq=R(80 / 255), vcf_env=R(45 / 255), env_a=0.0,
           env_d=R(60 / 255), env_s=0.0, env_r=R(60 / 255))),
    ("Overmono Pad", "Attack Magazine, Overmono-style pads: saw + half square + LFO PWM, high HPF; add pan delay and plate",
     patch(dco_pulse=True, dco_pwm=t(5), pwm_mode=PWM_LFO, dco_sub=t(3), dco_noise=t(1), dco_lfo=t(0.5), lfo_rate=t(2.5), hpf=HZ754, vcf_freq=t(7),
           env_a=0.0, env_d=1.0, env_s=0.5, env_r=t(5), chorus=I)),
]


# --- Output ---------------------------------------------------------------

def toml_value(x) -> str:
    if isinstance(x, bool):
        return "true" if x else "false"
    return repr(R(x) if isinstance(x, float) else x)


def write_preset(path: Path, name: str, author: str, comment: str, params: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = [f'uuid = "{uuid.uuid5(NS, f"{path.parent.name}/{name}")}"', f'name = "{esc(name)}"', f'author = "{author}"']
    if comment:
        lines.append(f'comment = "{esc(comment)}"')
    lines += ["", "[params]"]
    lines += [f"{IDS[k]} = {toml_value(x)}" for k, x in sorted(params.items(), key=lambda kv: IDS[kv[0]])]
    path.write_text("\n".join(lines) + "\n")


def esc(s: str) -> str:
    return s.replace("\\", "\\\\").replace('"', '\\"')


def slug(s: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")


def main() -> None:
    rows = re.findall(r'\{"([^"]+)",\s*\{([^}]*)\}\}', Path(sys.argv[1]).read_text())
    assert len(rows) == 256, len(rows)
    if OUT.exists():
        shutil.rmtree(OUT)

    write_preset(OUT / "Init.preset", "Init", "Jardim Sonoro", "Saw + pulse, filter open, Chorus I", dict(chorus=I))

    j60 = {}
    counts = {}
    for i, (name, vals) in enumerate(rows):
        v = [int(x) for x in vals.split(",")]
        # Unused slots, and Juno-60 B34 Drawbar Organ: its slider bytes are
        # all zero in the tape dump, so that patch is lost.
        if name.endswith(" Init") or not any(v[3:20]):
            continue
        code, title = name.split(" ", 1)
        if i < 128:
            params, group, author, notes = convert_j60(v), f"Juno-60 {code[0]}", "Roland (Juno-60 factory)", J60_NOTES
            j60[name] = params
        else:
            params, group, author, notes = convert_j106(v), f"Juno-106 {code[0]}", "Roland (Juno-106 factory)", J106_NOTES
        write_preset(OUT / group / f"{code}-{slug(title)}.preset", name, author, notes.get(name, ""), params)
        counts[group] = counts.get(group, 0) + 1

    missing = (set(J60_NOTES) | set(J106_NOTES)) - {name for name, _ in rows}
    assert not missing, missing

    for name, comment, spec in CLASSICS:
        if isinstance(spec, tuple):
            source, edits = spec
            spec = {**j60[source], **edits}
        write_preset(OUT / "Classics" / f"{slug(name)}.preset", name, "Jardim Sonoro", comment, spec)
    for name, comment, params in SOUND_DESIGN:
        write_preset(OUT / "Sound Design" / f"{slug(name)}.preset", name, "Jardim Sonoro", comment, params)

    print(counts, "classics", len(CLASSICS), "sound design", len(SOUND_DESIGN))


if __name__ == "__main__":
    main()
