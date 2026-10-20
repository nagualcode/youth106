#!/usr/bin/env python3
"""Generate presets/ from the Juno-106 factory bank.

Usage: gen_presets.py <ultramaster_kr106/Source/KR106_Presets_JUCE.h> <presets dir>

The KR-106 header carries the 128 Juno-106 factory patches decoded from
Roland's factory SysEx (raw 7-bit values, in KR-106 EParams order).
They are written as truce `.preset` files keyed by Youth106 field names,
in plain parameter units.

Conversions:
  - Sliders: byte / 127, except DCO PWM: byte / 105 (the J106 slider is
    limited to 105 by R5/R6, and 105 is the maximum pulse width).
  - DCO LFO: the patch byte is post analog taper (ADC reading); invert the
    taper to get the slider position (same as the KR-106 loader).
  - Chorus I / II bits -> Off / I / II / I+II.
  - The J106 has no sub switch: SUB OFF in the bank always has level 0.
"""
import math
import re
import sys
import uuid
from pathlib import Path

# KR-106 EParams indices used by the patch data.
DCO_LFO_IDX = 5


def inverse_taper(t_post: float) -> float:
    if t_post <= 0.0:
        return 0.0
    if t_post >= 1.0:
        return 1.0
    a = 5.0 * t_post
    b = 1.0 - 5.0 * t_post
    c = -t_post
    return (-b + math.sqrt(b * b - 4.0 * a * c)) / (2.0 * a)


def r3(x: float) -> float:
    return round(x, 4)


def convert(v: list[int]) -> dict:
    s = lambda i: r3(v[i] / 127.0)
    chorus = (1 if v[27] else 0) | (2 if v[28] else 0)
    return {
        "lfo_rate": s(3),
        "lfo_delay": s(4),
        "dco_range": v[29],
        "dco_lfo": r3(inverse_taper(v[DCO_LFO_IDX] / 127.0)),
        "dco_pwm": r3(min(v[6], 105) / 105.0),
        "pwm_mode": v[33],
        "dco_pulse": bool(v[23]),
        "dco_saw": bool(v[24]),
        "dco_sub": s(7) if v[25] else 0.0,
        "dco_noise": s(8),
        "hpf": v[9],
        "vcf_freq": s(10),
        "vcf_res": s(11),
        "vcf_env_pol": v[34],
        "vcf_env": s(12),
        "vcf_lfo": s(13),
        "vcf_kbd": s(14),
        "vca_mode": v[35],
        "vca_level": s(15),
        "env_a": s(16),
        "env_d": s(17),
        "env_s": s(18),
        "env_r": s(19),
        "chorus": chorus,
    }


def toml_value(x) -> str:
    if isinstance(x, bool):
        return "true" if x else "false"
    return repr(x)


def main() -> None:
    src = Path(sys.argv[1]).read_text()
    out = Path(sys.argv[2])
    rows = re.findall(r'\{"([^"]+)",\s*\{([^}]*)\}\}', src)
    assert len(rows) == 256, len(rows)
    bank = rows[128:]  # J106 bank
    ns = uuid.UUID("6f1d5c2a-1060-4a10-9b06-000000106106")
    for name, vals in bank:
        v = [int(x) for x in vals.split(",")]
        code, title = name.split(" ", 1)
        group = f"Factory {code[0]}"
        params = convert(v)
        d = out / group
        d.mkdir(parents=True, exist_ok=True)
        slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
        lines = [
            f'uuid = "{uuid.uuid5(ns, name)}"',
            f'name = "{code} {title}"',
            'author = "Roland (Juno-106 factory)"',
            "",
            "[params]",
        ]
        lines += [f"{k} = {toml_value(x)}" for k, x in params.items()]
        (d / f"{code}-{slug}.preset").write_text("\n".join(lines) + "\n")
    print(f"wrote {len(bank)} presets to {out}")


if __name__ == "__main__":
    main()
