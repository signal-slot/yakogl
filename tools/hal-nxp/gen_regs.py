#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Generate driver/rust/src/soc/imx8mp/regs.rs from hal_nxp's i.MX8MP PERI headers.

    tools/hal-nxp/gen_regs.py <hal_nxp checkout> > driver/rust/src/soc/imx8mp/regs.rs
    tools/hal-nxp/gen_regs.py --self-test

The input is NXP's CMSIS peripheral access layer for the i.MX 8M Plus,
`mcux/mcux-sdk-ng/devices/i.MX/i.MX8MP/periph/PERI_<BLOCK>.h` in Zephyr's
hal_nxp module, read unmodified (BSD-3-Clause, NXP; see NOTICE). From each
header it takes the register layout typedef (`<BLOCK>_Type`: register offsets,
and for arrays of register groups their base, step and count) and the
`<BLOCK>_<REGISTER>_<FIELD>_MASK` / `_SHIFT` macros with the value lists NXP
wrote beside them. Nothing is typed in by hand: a register or a bit that is
not in the header cannot appear in the output.

The inputs are pinned: each header's SHA-256 must equal the one recorded in
PINNED below, which were taken at hal_nxp revision HAL_NXP_REVISION. A header
that changed (a hal_nxp update) stops the generator, so regenerating against a
newer hal_nxp is a deliberate act: update the revision and the hashes, review
the diff of regs.rs, commit both. tools/check.sh regenerates and compares.

Only the blocks the Rust drivers use are generated (BLOCKS). Adding one is one
entry: its name, optionally the registers to keep (None = all), and its hash.

Python 3 standard library only.
"""
import hashlib
import os
import re
import sys

HAL_NXP_REVISION = "1e91007b164c52b74d58d4e75a856efbc5d4de05"
PERIPH_DIR = "mcux/mcux-sdk-ng/devices/i.MX/i.MX8MP/periph"

# block -> registers to keep (None: every register of the block)
BLOCKS = {
    "PWM": None,
    # The CCM header describes 39 PLL controls, 192 clock gates and 142 clock
    # roots with SET/CLR/TOG aliases each; the PWM needs one root and one gate.
    "CCM": ["CCGR", "CCGR_SET", "CCGR_CLR", "TARGET_ROOT", "TARGET_ROOT_SET", "TARGET_ROOT_CLR"],
    # The media power domains (soc::imx8mp::{gpc, anatop, media_blk}): the PU
    # power-up/-down triggers and their status, the ADB400 handshake and the
    # A53 mapping; the PU PGCs' control and status; VIDEO_PLL1 and the two
    # system PLLs the media clock roots run from; the MEDIAMIX resets, clock
    # enables, MIPI PHY resets and the LCDIF read-hurry.
    "GPC": ["PU_PGC_SW_PUP_REQ", "PU_PGC_SW_PDN_REQ", "A53_PU_PGC_PUP_STATUS", "A53_PU_PGC_PDN_STATUS",
            "PU_PWRHSK", "PGC_CPU_A53_MAPPING"],
    "GPC_PGC": ["PU_CTRL", "PU_SR"],
    "CCM_ANALOG": ["VIDEO_PLL1_GEN_CTRL", "VIDEO_PLL1_FDIV_CTL0", "VIDEO_PLL1_FDIV_CTL1",
                   "SYS_PLL1_GEN_CTRL", "SYS_PLL1_FDIV_CTL0", "SYS_PLL2_GEN_CTRL", "SYS_PLL2_FDIV_CTL0"],
    "MEDIA_BLK_CTRL": ["SFT_RSTN", "CLK_EN", "MIPI_RESET_DIV", "LCDIF_ARCACHE_CTRL"],
    # The Armadillo-X2's HDMI path (ip::lcdifv3, soc::imx8mp::hdmi,
    # ip::dw_hdmi_tx; driver/docs/x2-hdmi-cold.md): the LCDIFv3 (LCDIF3 is
    # the same IP at 0x32FC6000), the HDMIMIX block control, the parallel
    # video interface between LCDIF3 and the controller, and the parts of the
    # Synopsys DesignWare HDMI TX controller a fixed DVI-mode picture needs.
    # The controller's registers are 8 bits wide, one per byte address
    # (`REG_BITS`). The HDMI PHY (0x32FDFF00) has no hal_nxp header; its
    # registers are in ip/imx8mp_hdmi_phy.rs from IMX8MPRM Rev. 3 13.10.3.
    "LCDIF": None,
    "HDMI_TX_BLK_CTL": ["RTX_GENERAL", "RTX_RESET_CTL0", "RTX_RESET_CTL0_SET", "RTX_RESET_CTL0_CLR",
                        "RTX_CLK_CTL0", "RTX_CLK_CTL0_SET", "RTX_CLK_CTL0_CLR", "RTX_CLK_CTL1",
                        "RTX_CLK_CTL1_SET", "RTX_CLK_CTL1_CLR", "RTX_CLK_CTL2", "RTX_CLK_CTL3",
                        "TX_CONTROL0", "TX_CONTROL0_SET", "TX_CONTROL0_CLR", "TX_STATUS0"],
    "HTX_PVI": ["HTX_PVI_CTRL", "HTX_PVI_IRQ_MASK", "HTX_PVI_IRQ_STATUS", "HTX_PVI_IRQ_CLR",
                "HTX_TMG_GEN_DISP_LRC", "HTX_TMG_GEN_DE_ULC", "HTX_TMG_GEN_DE_LRC", "HTX_TMG_GEN_HSYNC",
                "HTX_TMG_GEN_VSYNC", "HTX_TMG_GEN_CFG"],
    "IDENTIFICATION": ["DESIGN_ID", "REVISION_ID", "PRODUCT_ID0", "PRODUCT_ID1"],
    "VIDEOSAMPLER": ["TX_INVID0", "TX_INSTUFFING"],
    "VIDEOPACKETIZER": ["VP_PR_CD", "VP_STUFF", "VP_REMAP", "VP_CONF"],
    "FRAMECOMPOSER": ["FC_INVIDCONF", "FC_INHACTIV0", "FC_INHACTIV1", "FC_INHBLANK0", "FC_INHBLANK1",
                      "FC_INVACTIV0", "FC_INVACTIV1", "FC_INVBLANK", "FC_HSYNCINDELAY0", "FC_HSYNCINDELAY1",
                      "FC_HSYNCINWIDTH0", "FC_HSYNCINWIDTH1", "FC_VSYNCINDELAY", "FC_VSYNCINWIDTH",
                      "FC_CTRLDUR", "FC_EXCTRLDUR", "FC_EXCTRLSPAC", "FC_CH0PREAM", "FC_CH1PREAM",
                      "FC_CH2PREAM", "FC_PRCONF", "FC_DBGFORCE"],
    "PHYCONFIGURATION": ["PHY_CONF0", "PHY_STAT0"],
    "MAINCONTROLLER": ["MC_CLKDIS", "MC_SWRSTZREQ_1", "MC_FLOWCTRL", "MC_PHYRSTZ", "MC_LOCKONCLOCK_1"],
    # The Samsung MIPI DSI host of the Verdin's LCDIF1 -> DSI path
    # (ip::samsung_dsim, driver/docs/imx8mp-display-cold.md): the registers a
    # video-mode link needs. The PLL's P/M/S split of PLLCTRL[PMS] is not in
    # the header (one 19-bit field); ip/samsung_dsim.rs states where it is from.
    "MIPI_DSI": ["DSI_VERSION", "DSI_STATUS", "DSI_SWRST", "DSI_CLKCTRL", "DSI_TIMEOUT", "DSI_CONFIG",
                 "DSI_ESCMODE", "DSI_MDRESOL", "DSI_MVPORCH", "DSI_MHPORCH", "DSI_MSYNC", "DSI_INTSRC",
                 "DSI_INTMSK", "DSI_FIFOCTRL", "DSI_PLLCTRL", "DSI_PLLTMR", "DSI_PHYCTRL_B1",
                 "DSI_PHYTIMING", "DSI_PHYTIMING1", "DSI_PHYTIMING2"],
}

PINNED = {
    "PERI_PWM.h": "5a85557a8a70ab85866f05559511f2365874d873a5bc388280303af55ef9f1a5",
    "PERI_CCM.h": "551d46e8db49c65e65df7f26a73ffa0cb07dfe79fe396df343ec1a57f74df161",
    "PERI_GPC.h": "e8e4b3f450308ce63e220ae915a78082e34e26b7545ac039700771158eaf5db7",
    "PERI_GPC_PGC.h": "327bace08b3e960b75d69b2b766317a97ff175e114f6b289ecf53a54c9fe3362",
    "PERI_CCM_ANALOG.h": "4ad57c190bb66a674b58d282a3cfdb2494699e64b05180d30145b0490f2aa4df",
    "PERI_MEDIA_BLK_CTRL.h": "84bc9c9236276066c277c73b32e3072373e8a2c16891fd69f92361c6bd3092a5",
    "PERI_LCDIF.h": "561cb80e3543bdce8d7292372beda8c57512b202ee977b37b80b436402cc12bd",
    "PERI_HDMI_TX_BLK_CTL.h": "ded21ec43ef3ec5d20ef984c6eb6ff17216ac898ae6e10738b319d563bb92dd0",
    "PERI_HTX_PVI.h": "96b242c061965c1c8ef50a611e4e3e3a6b7ee5c623e39169c5125722dbf3c013",
    "PERI_IDENTIFICATION.h": "4a3f98e3a3748d494f9deaa563a32c0cf33db8563658559c4c45d6c640528616",
    "PERI_VIDEOSAMPLER.h": "cd35003107907e3b807b3106aac2da33e00b818b78af9abbb27b86b5a2020573",
    "PERI_VIDEOPACKETIZER.h": "3440f5f10bff103896ce4b7dbde90543a1bf680370eab82164b4f4778fd2965f",
    "PERI_FRAMECOMPOSER.h": "71b90cd239f3096abfd98f1a051f8ccb371ec0415057b5a7a890ca5d44e9fb84",
    "PERI_PHYCONFIGURATION.h": "43f24f597ffb43a41f370f60de37b49f419da62c1e0e427634e2da11cccd14c3",
    "PERI_MAINCONTROLLER.h": "0b47effe17391cb764cbb056fbfde19d292d7c2b1f17c217c91488eccac37884",
    "PERI_MIPI_DSI.h": "94fceb991685e5a7607789ef28fb9e2f537bdd7287fbdb8b0f5e522c5db62c46",
}


class GenError(Exception):
    pass


MEMBER = re.compile(r"^\s*__(?:IO|I|O)\s+uint(?:32|8)_t\s+(\w+);\s*/\*\*<.*?(?:array offset|offset):\s*(0x[0-9A-Fa-f]+)")
# a plain array of registers: `__I uint32_t NAME[NAME_COUNT]; /**< ..., array offset: 0x124, array step: 0x4 */`
ARRAY_MEMBER = re.compile(r"^\s*__(?:IO|I|O)\s+uint32_t\s+(\w+)\[(\w+)\];\s*/\*\*<.*?array offset:\s*(0x[0-9A-Fa-f]+),\s*array step:\s*(0x[0-9A-Fa-f]+)")
RESERVED = re.compile(r"^\s*uint8_t\s+RESERVED_\w+\[\d+\];")
NESTED_OPEN = re.compile(r"^\s*struct\s*\{\s*/\*\s*offset:\s*(0x[0-9A-Fa-f]+),\s*array step:\s*(0x[0-9A-Fa-f]+)\s*\*/")
NESTED_CLOSE = re.compile(r"^\s*\}\s*(\w+)\[(\w+)\];")
# One register with SET/CLR/TOG aliases as a struct of its own (the i.MX 9
# header style some i.MX8MP headers use): `struct { /* offset: 0x20 */
# RW; SET; CLR; TOG } NAME;`. Emitted as NAME, NAME_SET, NAME_CLR, NAME_TOG.
GROUP_OPEN = re.compile(r"^\s*struct\s*\{\s*/\*\s*offset:\s*(0x[0-9A-Fa-f]+)\s*\*/\s*$")
GROUP_CLOSE = re.compile(r"^\s*\}\s*(\w+);")
COUNT = re.compile(r"^#define\s+(\w+_COUNT)\s+(\d+)u\s*$")
MASK = re.compile(r"^#define\s+(\w+)_MASK\s+\((0x[0-9A-Fa-f]+)U\)")
SHIFT = re.compile(r"^#define\s+(\w+)_SHIFT\s+\((\d+)U\)")
DOC = re.compile(r"^\s*\*\s+(0b[01]+\.\..*?)\s*$")


def parse(block, text):
    """-> (registers, arrays, fields).

    registers: [(name, offset)] of plain registers;
    arrays: [(name, base, step, count, [(member, offset_in_element)])];
    fields: {register: [(field, mask, shift, [doc lines])]};
    bits: the register width, 32 or 8 (a block of 8-bit registers, one per
    byte address: the Synopsys HDMI TX controller's). A block that mixes
    widths is refused.
    """
    lines = text.splitlines()
    widths = set()
    counts = {}
    for ln in lines:
        m = COUNT.match(ln)
        if m:
            counts[m.group(1)] = int(m.group(2))
    try:
        start = lines.index("typedef struct {")
    except ValueError:
        raise GenError(f"{block}: no register layout typedef")
    registers, arrays = [], []
    nested = None
    group = None
    i = start + 1
    while True:
        if i >= len(lines):
            raise GenError(f"{block}: the typedef does not end")
        ln = lines[i]
        i += 1
        if re.match(rf"^\}}\s*{block}_Type;", ln):
            break
        m = GROUP_OPEN.match(ln)
        if m and nested is None:
            if group is not None:
                raise GenError(f"{block}: register group inside a register group")
            group = []
            continue
        m = GROUP_CLOSE.match(ln)
        if m and group is not None:
            name = m.group(1)
            for member, off in group:
                if member not in ("RW", "SET", "CLR", "TOG"):
                    raise GenError(f"{block}: {name}.{member}: not an RW/SET/CLR/TOG group")
                registers.append((name if member == "RW" else f"{name}_{member}", off))
            group = None
            continue
        m = NESTED_OPEN.match(ln)
        if m:
            if nested is not None:
                raise GenError(f"{block}: nested struct inside a nested struct")
            nested = (int(m.group(1), 16), int(m.group(2), 16), [])
            continue
        m = NESTED_CLOSE.match(ln)
        if m:
            if nested is None:
                raise GenError(f"{block}: unbalanced struct close: {ln.strip()}")
            name, count_macro = m.group(1), m.group(2)
            if count_macro not in counts:
                raise GenError(f"{block}: no {count_macro}")
            base, step, members = nested
            arrays.append((name, base, step, counts[count_macro], [(n, o - base) for n, o in members]))
            nested = None
            continue
        m = MEMBER.match(ln)
        if m:
            widths.add(8 if "uint8_t" in ln.split(m.group(1))[0] else 32)
            if group is not None:
                group.append((m.group(1), int(m.group(2), 16)))
                continue
            (nested[2] if nested is not None else registers).append((m.group(1), int(m.group(2), 16)))
            continue
        m = ARRAY_MEMBER.match(ln)
        if m and nested is None:
            name, count_macro = m.group(1), m.group(2)
            if count_macro not in counts:
                raise GenError(f"{block}: no {count_macro}")
            # one register per element: an array of groups of one member
            arrays.append((name, int(m.group(3), 16), int(m.group(4), 16), counts[count_macro], [(name, 0)]))
            continue
        if RESERVED.match(ln) or not ln.strip():
            continue
        raise GenError(f"{block}: line the generator does not understand: {ln.strip()}")

    reg_names = {n for n, _ in registers}
    for _, _, _, _, members in arrays:
        reg_names |= {n for n, _ in members}
    by_len = sorted(reg_names, key=len, reverse=True)

    fields = {}
    shifts = {}
    pending = {}
    for idx, ln in enumerate(lines):
        m = SHIFT.match(ln)
        if m:
            shifts[m.group(1)] = int(m.group(2))
            continue
        m = MASK.match(ln)
        if not m or not m.group(1).startswith(block + "_"):
            continue
        full = m.group(1)[len(block) + 1:]
        reg = next((r for r in by_len if full.startswith(r + "_")), None)
        if reg is None:
            raise GenError(f"{block}: mask {m.group(1)} names no register of the typedef")
        doc = []
        j = idx + 1
        # skip the SHIFT line, then collect a /*! FIELD ... */ value list
        while j < len(lines) and lines[j].startswith("#define") and lines[j].find("_SHIFT") >= 0:
            j += 1
        if j < len(lines) and lines[j].lstrip().startswith("/*!"):
            j += 1
            while j < len(lines) and not lines[j].strip().startswith("*/"):
                d = DOC.match(lines[j])
                if d:
                    doc.append(d.group(1))
                j += 1
        # NXP names some fields in lower case (the 8-bit HDMI blocks); Rust
        # constants are upper case, and no two fields may then collide.
        pending[m.group(1)] = (reg, full[len(reg) + 1:].upper(), int(m.group(2), 16), doc)
    seen = set()
    for reg, field, _, _ in pending.values():
        if (reg, field) in seen:
            raise GenError(f"{block}: field {reg}.{field} appears twice")
        seen.add((reg, field))
    if len(widths) != 1:
        raise GenError(f"{block}: register widths {sorted(widths)}; one width per block")
    for macro, (reg, field, mask, doc) in pending.items():
        if macro not in shifts:
            raise GenError(f"{block}: {macro}_MASK has no _SHIFT")
        shift = shifts[macro]
        if mask == 0 or (mask >> shift) << shift != mask:
            raise GenError(f"{block}: {macro}: mask 0x{mask:x} does not start at bit {shift}")
        fields.setdefault(reg, []).append((field, mask, shift, doc))
    return registers, arrays, fields, widths.pop()


def emit_fields(out, reg, fields, indent):
    if reg not in fields:
        return
    pad = " " * indent
    out.append(f"{pad}/// Fields of `{reg}`.")
    out.append(f"{pad}pub mod {reg.lower()} {{")
    for field, mask, shift, doc in fields[reg]:
        for d in doc:
            out.append(f"{pad}    /// {d}")
        out.append(f"{pad}    pub const {field}_MASK: u32 = 0x{mask:x};")
        out.append(f"{pad}    pub const {field}_SHIFT: u32 = {shift};")
    out.append(f"{pad}}}")


def generate(hal):
    out = [
        "// SPDX-License-Identifier: BSD-3-Clause",
        "// GENERATED by tools/hal-nxp/gen_regs.py. DO NOT EDIT.",
        "//",
        "// Register offsets and fields of the i.MX 8M Plus, from NXP's CMSIS",
        f"// peripheral headers in Zephyr's hal_nxp module ({PERIPH_DIR}),",
        f"// revision {HAL_NXP_REVISION}:",
    ]
    for name in BLOCKS:
        out.append(f"//   PERI_{name}.h  sha256 {PINNED['PERI_' + name + '.h']}")
    out += [
        "// Copyright 1997-2016 Freescale Semiconductor, Inc.; Copyright 2016-2026 NXP.",
        "// The field value lists are NXP's text from the same headers.",
        "",
        "#![allow(clippy::identity_op)]",
    ]
    for block, keep in BLOCKS.items():
        fname = f"PERI_{block}.h"
        path = os.path.join(hal, PERIPH_DIR, fname)
        try:
            data = open(path, "rb").read()
        except OSError as e:
            raise GenError(f"cannot read {path}: {e}")
        digest = hashlib.sha256(data).hexdigest()
        if digest != PINNED[fname]:
            raise GenError(f"{fname}: sha256 {digest} is not the pinned {PINNED[fname]} "
                           f"(hal_nxp {HAL_NXP_REVISION}); update the pin on purpose")
        registers, arrays, fields, bits = parse(block, data.decode("utf-8"))
        kept = set(keep) if keep is not None else None
        if kept is not None:
            known = {n for n, _ in registers} | {n for a in arrays for n, _ in a[4]}
            missing = kept - known
            if missing:
                raise GenError(f"{block}: no register(s) {sorted(missing)}")
        out.append("")
        out.append(f"/// `{block}_Type` (PERI_{block}.h): offsets from the block's base address.")
        out.append(f"pub mod {block.lower()} {{")
        first = True
        if bits != 32:
            out.append(f"    /// Every register of the block is {bits} bits wide, one per byte address.")
            out.append(f"    pub const REG_BITS: u32 = {bits};")
            first = False
        for name, off in registers:
            if kept is not None and name not in kept:
                continue
            if not first:
                out.append("")
            first = False
            out.append(f"    pub const {name}: u32 = 0x{off:x};")
            emit_fields(out, name, fields, 4)
        for name, base, step, count, members in arrays:
            members = [(n, o) for n, o in members if kept is None or n in kept]
            if not members:
                continue
            if not first:
                out.append("")
            first = False
            out.append(f"    /// `{name}[{count}]`: register groups of {step:#x} bytes from {base:#x}.")
            out.append(f"    pub const {name}_BASE: u32 = 0x{base:x};")
            out.append(f"    pub const {name}_STEP: u32 = 0x{step:x};")
            out.append(f"    pub const {name}_COUNT: u32 = {count};")
            for mname, moff in members:
                out.append(f"    /// `{name}[n].{mname}`.")
                out.append(f"    pub const fn {mname.lower()}(n: u32) -> u32 {{")
                out.append(f"        {name}_BASE + n * {name}_STEP + 0x{moff:x}")
                out.append("    }")
            for mname, _ in members:
                emit_fields(out, mname, fields, 4)
        out.append("}")
    return "\n".join(out) + "\n"


SAMPLE = """\
#define XX_A_COUNT 3u
#define XX_S_COUNT 2u
typedef struct {
  __IO uint32_t CR;                             /**< Control, offset: 0x0 */
       uint8_t RESERVED_0[12];
  struct {                                         /* offset: 0x10, array step: 0x8 */
    __IO uint32_t A;                              /**< A, array offset: 0x10, array step: 0x8 */
    __IO uint32_t A_SET;                          /**< A, array offset: 0x14, array step: 0x8 */
  } A[XX_A_COUNT];
  __I  uint32_t S[XX_S_COUNT];                    /**< S, array offset: 0x28, array step: 0x4 */
} XX_Type;
#define XX_CR_EN_MASK                        (0x1U)
#define XX_CR_EN_SHIFT                       (0U)
/*! EN
 *  0b0..off
 *  0b1..on
 */
#define XX_A_SET_V_MASK                      (0x30U)
#define XX_A_SET_V_SHIFT                     (4U)
#define XX_A_V_MASK                          (0x30U)
#define XX_A_V_SHIFT                         (4U)
#define XX_S_F_MASK                          (0x2U)
#define XX_S_F_SHIFT                         (1U)
"""


SAMPLE8 = """\
typedef struct {
  __IO uint8_t B_CONF;                          /**< Conf, offset: 0x0 */
       uint8_t RESERVED_0[2];
  __I  uint8_t B_STAT;                          /**< Stat, offset: 0x3 */
} YY_Type;
#define YY_B_CONF_low_name_MASK              (0x6U)
#define YY_B_CONF_low_name_SHIFT             (1U)
"""


def self_test():
    regs, arrays, fields, bits = parse("XX", SAMPLE)
    ok = bits == 32
    ok &= regs == [("CR", 0)]
    ok &= arrays == [("A", 0x10, 8, 3, [("A", 0), ("A_SET", 4)]), ("S", 0x28, 4, 2, [("S", 0)])]
    ok &= fields["S"] == [("F", 2, 1, [])]
    ok &= fields["CR"] == [("EN", 1, 0, ["0b0..off", "0b1..on"])]
    # the longest register name wins: A_SET_V is field V of A_SET, not SET_V of A
    ok &= fields["A_SET"] == [("V", 0x30, 4, [])] and fields["A"] == [("V", 0x30, 4, [])]
    # 8-bit registers at byte offsets; NXP's lower-case field names upper-cased
    regs8, _, fields8, bits8 = parse("YY", SAMPLE8)
    ok &= bits8 == 8 and regs8 == [("B_CONF", 0), ("B_STAT", 3)]
    ok &= fields8["B_CONF"] == [("LOW_NAME", 6, 1, [])]
    mixed = SAMPLE8.replace("__I  uint8_t B_STAT", "__I  uint32_t B_STAT")
    for blk, bad in (("XX", SAMPLE.replace("(0x30U)", "(0x31U)", 1)),
                     ("XX", SAMPLE.replace("  __IO uint32_t CR;", "  __IO uint16_t CR;")),
                     ("YY", mixed)):
        try:
            parse(blk, bad)
            ok = False
        except GenError:
            pass
    print("gen_regs self-test:", "ok" if ok else "FAILED")
    return 0 if ok else 1


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if len(argv) != 2:
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 2
    try:
        sys.stdout.write(generate(argv[1]))
    except GenError as e:
        print(f"gen_regs: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
