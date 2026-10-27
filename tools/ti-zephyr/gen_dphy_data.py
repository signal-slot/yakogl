#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Generate driver/rust/src/soc/am62p/ti_pr114866.rs from TI's vendored Zephyr DSI driver.

    tools/ti-zephyr/gen_dphy_data.py > driver/rust/src/soc/am62p/ti_pr114866.rs
    tools/ti-zephyr/gen_dphy_data.py --check     (regenerate and compare; tools/check.sh)

The AM62P's D-PHY TX and the Cadence DSI host are written from the AM62Px TRM
(TI SPRUJ83D) in driver/rust/src/ip/cdns_{dphy,dsi}.rs. Four facts the driver
needs are not in that TRM (driver/docs/am62p-display-cold.md, "What the TRM
documents"):

  1. the data-rate bands of PCS_TX_DIG_TBIT0 PCS_BAND_CTL_REG_L/_R (the TRM
     names band 0 only, "Data Rate [80_100] MHz");
  2. the values for CMN_DIG_TBIT10 (the PLL gm PWM dividers; the TRM gives the
     fields, not what to write);
  3. where the WIZ wrapper's registers sit inside the D-PHY region (the TRM's
     DPHY_TX summary puts them at offsets 0h-14h, which overlap nothing but
     disagree with TI's own driver);
  4. the MCTL_DPHY_CFG0 sequence (the TRM says each bit "drives dphy_*_pdn /
     _pso / _rstb output", not their polarity or order).

Per docs/source-form.md group E step 7, where the TRM is silent the source is
TI's Zephyr driver of PR #114866, vendored unmodified at
third_party/zephyr-pr114866-2026-07-29-2473da6b/ (Apache-2.0, Texas
Instruments). This generator reads exactly those four facts out of that file
-- the `cdns_dphy_tx_bands[]` table, the `DPHY_CMN_PWM_*()` call, the
`DPHY_TX_WIZ_*` offsets, the `MCTL_DPHY_CFG0` writes of `dsi_hs_init()` -- and
writes them as Rust constants. Nothing is typed in by hand: if the vendored
file changes shape, the generator stops.

The input is pinned by SHA-256 (PINNED below). Python 3 standard library only.
"""
import hashlib
import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
VENDOR_DIR = "third_party/zephyr-pr114866-2026-07-29-2473da6b"
SOURCE = "drivers/mipi_dsi/dsi_cdns_ti.c"
PINNED = "1d09395d1eba4a00be605b34c5720f1ba4851932747f391f04ce39bcb9e5ca9a"
OUTPUT = "driver/rust/src/soc/am62p/ti_pr114866.rs"


def fail(msg):
    sys.stderr.write("gen_dphy_data.py: %s\n" % msg)
    sys.exit(1)


def read_source():
    path = os.path.join(ROOT, VENDOR_DIR, SOURCE)
    data = open(path, "rb").read()
    got = hashlib.sha256(data).hexdigest()
    if got != PINNED:
        fail("%s has sha256 %s, pinned %s: a different file is not regenerated silently" % (path, got, PINNED))
    return data.decode("utf-8")


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", " ", text)


def defines(text):
    """Object-like and one-parameter function-like #defines: name -> (param, body)."""
    out = {}
    for m in re.finditer(r"^#define\s+(\w+)(\((\w+)\))?\s+(.*)$", text, flags=re.M):
        out[m.group(1)] = (m.group(3), m.group(4).strip())
    return out


def split_args(arg):
    """Split a macro argument list at its top-level commas."""
    out, level, cur = [], 0, ""
    for c in arg:
        if c == "," and level == 0:
            out.append(cur)
            cur = ""
            continue
        level += c == "("
        level -= c == ")"
        cur += c
    return out + [cur]


class Evaluator:
    """Integer value of a C constant expression over the file's own macros:
    BIT(), GENMASK(), shifts, |, +, -, parentheses, U suffixes."""

    def __init__(self, macros):
        self.macros = macros

    def expand(self, expr, depth=0):
        if depth > 20:
            fail("macro expansion too deep in %r" % expr)
        expr = re.sub(r"\b(0[xX][0-9a-fA-F]+|\d+)[uU][lL]*\b", r"\1", expr)

        def call(name, arg):
            if name == "BIT":
                return "(1 << (%s))" % self.expand(arg, depth + 1)
            if name == "GENMASK":
                hi, lo = [self.expand(a, depth + 1) for a in split_args(arg)]
                return "((((1 << ((%s) - (%s) + 1)) - 1)) << (%s))" % (hi, lo, lo)
            if name in self.macros and self.macros[name][0]:
                param, body = self.macros[name]
                return "(%s)" % self.expand(re.sub(r"\b%s\b" % param, "(%s)" % arg, body), depth + 1)
            fail("unknown macro call %s(%s)" % (name, arg))

        # Replace the first macro call (balanced parentheses) until none is left.
        while True:
            m = re.search(r"\b([A-Za-z_]\w*)\(", expr)
            if not m:
                break
            i, level = m.end(), 1
            while level:
                if i >= len(expr):
                    fail("unbalanced parentheses in %r" % expr)
                level += expr[i] == "("
                level -= expr[i] == ")"
                i += 1
            expr = expr[:m.start()] + call(m.group(1), expr[m.end():i - 1]) + expr[i:]

        def word(m):
            name = m.group(0)
            if name in self.macros and not self.macros[name][0]:
                return "(%s)" % self.expand(self.macros[name][1], depth + 1)
            return name

        return re.sub(r"\b[A-Za-z_]\w*\b", word, expr)

    def value(self, expr):
        e = self.expand(expr)
        if not re.fullmatch(r"[\s0-9a-fA-FxX()<>|+\-]*", e):
            fail("not a constant expression: %r -> %r" % (expr, e))
        return eval(e, {"__builtins__": {}}, {})  # noqa: S307 (digits and operators only)


def function_body(text, name):
    m = re.search(r"\b%s\s*\([^)]*\)\s*\{" % re.escape(name), text)
    if not m:
        fail("function %s not found" % name)
    depth, i = 1, m.end()
    while depth:
        c = text[i]
        depth += c == "{"
        depth -= c == "}"
        i += 1
    return text[m.end():i - 1]


def main():
    check = sys.argv[1:] == ["--check"]
    if sys.argv[1:] not in ([], ["--check"]):
        fail("usage: gen_dphy_data.py [--check]")
    src = read_source()
    text = strip_comments(src)
    macros = defines(text)
    ev = Evaluator(macros)

    # 1. the band table
    m = re.search(r"cdns_dphy_tx_bands\[\]\s*=\s*\{([^}]*)\}", text)
    if not m:
        fail("cdns_dphy_tx_bands[] not found")
    bands = [int(v, 0) for v in re.findall(r"\d+", m.group(1))]
    if len(bands) < 2 or bands != sorted(bands):
        fail("band table not ascending: %r" % bands)

    # 2. the CMN PWM write in dphy_power_on()
    body = function_body(text, "dphy_power_on")
    m = re.search(r"DPHY_CMN_PWM\s*,\s*DPHY_CMN_PWM_HIGH\(([^)]*)\)\s*\|\s*DPHY_CMN_PWM_LOW\(([^)]*)\)"
                  r"\s*\|\s*DPHY_CMN_PWM_DIV\(([^)]*)\)\)", body)
    if not m:
        fail("the DPHY_CMN_PWM write of dphy_power_on() not found")
    pwm_high, pwm_low, pwm_div = (ev.value(g) for g in m.groups())
    pwm_off = ev.value("DPHY_CMN_PWM")
    pwm_shifts = tuple(ev.value("%s(1)" % n).bit_length() - 1
                       for n in ("DPHY_CMN_PWM_HIGH", "DPHY_CMN_PWM_LOW", "DPHY_CMN_PWM_DIV"))

    # 3. the WIZ registers
    wiz = {n: ev.value("DPHY_TX_WIZ_" + n) for n in ("PLL_CTRL", "STATUS", "RST_CTRL", "PSM_FREQ")}

    # 4. the MCTL_DPHY_CFG0 writes of dsi_hs_init(), in order
    body = function_body(text, "dsi_hs_init")
    writes = re.findall(r"dsi_wr\(\s*dev\s*,\s*MCTL_DPHY_CFG0\s*,(.*?)\)\s*;", body, flags=re.S)
    if len(writes) != 3:
        fail("expected three MCTL_DPHY_CFG0 writes in dsi_hs_init(), found %d" % len(writes))
    cfg0 = []
    lane_rstb_in = None
    for i, w in enumerate(writes):
        w = " ".join(w.split())
        if "DPHY_D_RSTB(nlanes)" in w:
            lane_rstb_in = i
            w = w.replace("DPHY_D_RSTB(nlanes) |", "").replace("| DPHY_D_RSTB(nlanes)", "")
        cfg0.append(ev.value(w))
    if lane_rstb_in != 2:
        fail("DPHY_D_RSTB(nlanes) expected in the third MCTL_DPHY_CFG0 write")
    d_rstb_lane0 = ev.value("DPHY_D_RSTB(1)")

    out = []
    w = out.append
    w("// SPDX-License-Identifier: Apache-2.0")
    w("// SPDX-FileCopyrightText: 2026 Texas Instruments Incorporated (the values below)")
    w("//! GENERATED by tools/ti-zephyr/gen_dphy_data.py -- do not edit; tools/check.sh")
    w("//! regenerates and compares.")
    w("//!")
    w("//! The facts the D-PHY TX / DSI drivers need that the AM62Px TRM (SPRUJ83D)")
    w("//! does not state, read from TI's Zephyr driver of PR #114866, vendored")
    w("//! unmodified at %s/%s" % (VENDOR_DIR, SOURCE))
    w("//! (Apache-2.0; sha256 %s)." % PINNED)
    w("//! ip::cdns_dphy and ip::cdns_dsi say where each is used; the generator's")
    w("//! header says why each one is here.")
    w("")
    w("/// `cdns_dphy_tx_bands[]`: the lower edge of each PCS band, in Mbit/s per")
    w("/// lane. Band `i` covers `[TX_BANDS_MBPS[i], TX_BANDS_MBPS[i + 1])`; the TRM")
    w("/// (14.8.6.3.2.10) names band 0 only, \"Data Rate [80_100] MHz\".")
    w("pub const TX_BANDS_MBPS: [u32; %d] = [" % len(bands))
    for i in range(0, len(bands), 12):
        w("    " + " ".join("%d," % b for b in bands[i:i + 12]))
    w("];")
    w("")
    w("/// The `DPHY_CMN_PWM` write of `dphy_power_on()`: register offset and the")
    w("/// three values, with the shifts the file's macros put them at (the TRM's")
    w("/// CMN0_O_ANA_PLL_GM_PWM_DIV_HIGH 9:0, _DIV_LOW 19:10, _BYTECLK_DIV 27:20 of")
    w("/// DPHY_TX_DPHYTX_CMN0_CMN_DIG_TBIT10, 14.8.6.3.2.2; checked in ip::cdns_dphy).")
    w("pub const CMN_PWM_OFFSET: u32 = %#x;" % pwm_off)
    w("pub const CMN_PWM_DIV_HIGH: u32 = %#x;" % pwm_high)
    w("pub const CMN_PWM_DIV_LOW: u32 = %#x;" % pwm_low)
    w("pub const CMN_PWM_BYTECLK_DIV: u32 = %#x;" % pwm_div)
    w("pub const CMN_PWM_SHIFTS: [u32; 3] = [%d, %d, %d];" % pwm_shifts)
    w("")
    w("/// `DPHY_TX_WIZ_*`: the WIZ registers' offsets in the D-PHY region (the TRM's")
    w("/// DPHY_TX_WIZ_CONFIG_PLL_CTRL / _STATUS / _RST_CTRL / _PSM_FREQ, which its")
    w("/// summary table lists at 4h / 8h / Ch / 10h).")
    w("pub const WIZ_PLL_CTRL: u32 = %#x;" % wiz["PLL_CTRL"])
    w("pub const WIZ_STATUS: u32 = %#x;" % wiz["STATUS"])
    w("pub const WIZ_RST_CTRL: u32 = %#x;" % wiz["RST_CTRL"])
    w("pub const WIZ_PSM_FREQ: u32 = %#x;" % wiz["PSM_FREQ"])
    w("")
    w("/// The three `MCTL_DPHY_CFG0` values of `dsi_hs_init()`, in order: before the")
    w("/// D-PHY PLL is programmed, after it (wait for PLL_LCK), and to release the")
    w("/// lanes; the third is OR-ed with `DPHY_D_RSTB(lanes)`, one bit per lane from")
    w("/// [`CFG0_D_RSTB_LANE0`] up.")
    w("pub const CFG0_SEQUENCE: [u32; 3] = [%#x, %#x, %#x];" % tuple(cfg0))
    w("pub const CFG0_D_RSTB_LANE0: u32 = %#x;" % d_rstb_lane0)
    text_out = "\n".join(out) + "\n"

    if check:
        path = os.path.join(ROOT, OUTPUT)
        if open(path).read() != text_out:
            fail("%s differs from the generator's output: regenerate it" % OUTPUT)
        return
    sys.stdout.write(text_out)


if __name__ == "__main__":
    main()
