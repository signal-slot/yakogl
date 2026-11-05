#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The partial render's background-object load shader (tools/pco_shc/spm_load/)
as generator output.

Mesa's PowerVR driver generates this shader at run time with pco
(pvr_uscgen_spm_load, src/imagination/vulkan/pvr_usc.c): the render that
resumes after a partial render runs it as the background object, so every
pixel's output registers start as the tile the partial render stored to the
SPM scratch. We have no pco on the target, so it is generated once here, for
the one configuration the backend uses (PROPS below: 1 output dword, no tile
buffers, 1 sample; the BXS-4-64), and committed with its metadata. A later
change turns the committed words into the backend's table entry.

Two steps, so that the second needs no Mesa build:

  gen_spm_load.py capture <capture_spm_load> [--check]
      Runs Mesa 26.2.1's pco through tools/pco_shc/capture_spm_load.c (built
      in a Mesa tree, README.md) and writes tools/pco_shc/spm_load/
      spm_load.hex (the machine code) and spm_load.meta.txt (what the
      compiler says about it). --check compares instead of writing: the
      regenerate-and-compare row of tools/check.sh.

  gen_spm_load.py check
      Disassembles the committed words with `yakogl-tools usc-dis` (the
      online compiler's decoder, src/shaderc/online/powervr; YAKOGL_TOOLS
      names a built binary, else `cargo run -p yakogl-tools`) and checks
      them against the metadata: the size, that the groups tile the whole
      program, that the one `end` is on the last group, that every shared
      register the program reads lies in the block the PDS program DMAs
      (consts_dwords) and both state slots are read, that every temp lies
      below `temps`, and that the last group writes pixel output 0 with
      the output-register check. No Mesa build; the decode row of
      tools/check.sh.

Run from the repository root.
"""
import hashlib
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT_DIR = os.path.join(ROOT, "tools/pco_shc/spm_load")
HEX = os.path.join(OUT_DIR, "spm_load.hex")
META = os.path.join(OUT_DIR, "spm_load.meta.txt")
SCRATCH = os.path.join(ROOT, "build/spm_load")

# struct pvr_spm_load_props (pvr_usc.h): output_reg_count, tile_buffer_count,
# is_multisampled. The backend renders to one RGBA8 colour target (one output
# dword), never to tile buffers, with one sample.
PROPS = ("1", "0", "0")
MESA = "26.2.1"
DEVICE = "BXS-4-64 (BVNC 36.53.104.796, TI AM62P)"
# Mesa's FRAG_RESULT_DATA0 (compiler/shader_enums.h): the colour output.
FRAG_RESULT_DATA0 = 4
# The special-register index of pixel output 0 (pco_isa.py: pixout0 = 32).
SPECIAL_PIXOUT0 = 32
# ROGUE_NUM_TEXSTATE_DWORDS (hwdef/rogue_hw_defs.h): a sampler or image state.
TEXSTATE_DWORDS = 4

HEADER = [
    "# SPDX-License-Identifier: Apache-2.0 AND MIT",
    "# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.",
    "# SPDX-FileCopyrightText: 2025 Imagination Technologies Ltd.",
    f"# Mesa {MESA} pco, {DEVICE}: the SPM (partial render)",
    "# background-object load shader, pvr_uscgen_spm_load() of",
    "# src/imagination/vulkan/pvr_usc.c (MIT) with struct pvr_spm_load_props",
    f"# {{ output_reg_count {PROPS[0]}, tile_buffer_count {PROPS[1]}, is_multisampled {PROPS[2]} }}.",
    "# Written by: tools/pco_shc/gen_spm_load.py capture <mesa>/build/src/imagination/vulkan/capture_spm_load",
    "# (tools/pco_shc/capture_spm_load.c, README.md). Do not edit.",
]


def capture_run(capture_spm_load):
    # pco prints its debug listings on stdout, where the capture's lines go.
    env = {k: v for k, v in os.environ.items() if not k.startswith("PCO_DEBUG")}
    out = subprocess.run([capture_spm_load, *PROPS], check=True, capture_output=True, text=True, env=env).stdout
    kv = []
    words = None
    for line in out.splitlines():
        key, _, value = line.partition(" ")
        if key == "words":
            words = [int(w, 16) for w in value.split()]
        else:
            kv.append((key, value))
    if words is None:
        sys.exit("capture_spm_load printed no words line")
    return kv, words


def words_bytes(words):
    return b"".join(w.to_bytes(4, "little") for w in words)


def render_hex(words):
    lines = HEADER + [f"# {len(words)} dwords ({len(words) * 4} bytes), little-endian, in program order."]
    for i in range(0, len(words), 4):
        lines.append(" ".join(f"0x{w:08x}" for w in words[i:i + 4]))
    return "\n".join(lines) + "\n"


def render_meta(kv, words):
    lines = HEADER + [
        "# One `key value` line each, as capture_spm_load prints them; the shared-",
        "# register slots are enum pvr_spm_load_data (src/imagination/common/pvr_iface.h).",
        "program spm_load",
        f"mesa {MESA}",
        "device 36.53.104.796",
        f"capture_args {' '.join(PROPS)}",
    ]
    lines += [f"{k} {v}" for k, v in kv]
    lines.append(f"usc_dwords {len(words)}")
    lines.append(f"sha256 {hashlib.sha256(words_bytes(words)).hexdigest()}")
    return "\n".join(lines) + "\n"


def capture(capture_spm_load, check):
    kv, words = capture_run(capture_spm_load)
    files = ((HEX, render_hex(words)), (META, render_meta(kv, words)))
    if check:
        bad = 0
        for path, text in files:
            rel = os.path.relpath(path, ROOT)
            if not os.path.exists(path):
                print(f"{rel}: missing")
                bad += 1
            elif open(path).read() != text:
                print(f"{rel}: differs from what this pco build captures")
                bad += 1
        if bad:
            sys.exit(1)
        print(f"gen_spm_load: spm_load.hex ({len(words)} dwords) and spm_load.meta.txt = this pco build's output")
    else:
        os.makedirs(OUT_DIR, exist_ok=True)
        for path, text in files:
            open(path, "w").write(text)


def read_hex():
    words = []
    header = []
    for line in open(HEX):
        line = line.strip()
        if not line:
            continue
        if line.startswith("#"):
            header.append(line)
            continue
        words += [int(w, 16) for w in line.split()]
    return header, words


def read_meta():
    meta = {}
    for line in open(META):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, _, value = line.partition(" ")
        meta.setdefault(key, []).append(value)
    return meta


def one(meta, key):
    if key not in meta or len(meta[key]) != 1:
        sys.exit(f"spm_load.meta.txt: expected exactly one `{key}` line")
    return meta[key][0]


def fields(value):
    """'a=1 b=2' -> {'a': '1', 'b': '2'}."""
    return dict(f.split("=", 1) for f in value.split())


def usc_dis(path):
    """The listing `yakogl-tools usc-dis <path>` prints, as its lines."""
    tool = os.environ.get("YAKOGL_TOOLS")
    cmd = [tool] if tool else ["cargo", "run", "-q", "-p", "yakogl-tools", "--"]
    r = subprocess.run([*cmd, "usc-dis", path], capture_output=True, text=True, cwd=ROOT)
    if r.returncode != 0:
        sys.exit(f"{' '.join(cmd)} usc-dis {os.path.relpath(path, ROOT)} failed:\n{r.stdout}{r.stderr}")
    return r.stdout.splitlines()


def parse_listing(lines):
    """[(offset, size, kind, {'hdr': {...}, 'src': [(bank, index)...], 'dst': [(bank, index)...]})]."""
    groups = []
    for line in lines:
        m = re.match(r"\[([0-9a-f]+)\] (\d+) bytes, (\w+)", line)
        if m:
            groups.append((int(m.group(1), 16), int(m.group(2)), m.group(3), {"hdr": {}, "src": [], "dst": []}))
            continue
        if not groups:
            continue
        g = groups[-1][3]
        s = line.strip()
        if s.startswith("hdr:"):
            g["hdr"] = fields(s.split("]", 1)[1])
        elif s.startswith(("lower_src:", "upper_src:")):
            g["src"] += re.findall(r"sb\d=(\w+) s\d=(\d+)", s)
        elif s.startswith("dst:"):
            g["dst"] += re.findall(r"db\w=(\w+) d\w=(\d+)", s)
    return groups


def check():
    header, words = read_hex()
    meta = read_meta()
    bad = []

    def expect(cond, what):
        if not cond:
            bad.append(what)

    text = "\n".join(header)
    expect(f"Mesa {MESA}" in text and "36.53.104.796" in text, "spm_load.hex header names Mesa 26.2.1 and the device")
    expect(f"output_reg_count {PROPS[0]}, tile_buffer_count {PROPS[1]}, is_multisampled {PROPS[2]}" in text,
           "spm_load.hex header names the inputs (PROPS)")
    expect(one(meta, "mesa") == MESA and one(meta, "device") == "36.53.104.796", "spm_load.meta.txt names Mesa 26.2.1 and the device")
    expect(one(meta, "capture_args") == " ".join(PROPS), "spm_load.meta.txt capture_args = PROPS")
    props = fields(one(meta, "props"))
    expect((props.get("output_reg_count"), props.get("tile_buffer_count"), props.get("is_multisampled")) == PROPS,
           "spm_load.meta.txt props = PROPS")

    data = words_bytes(words)
    expect(len(words) > 0, "spm_load.hex holds words")
    expect(int(one(meta, "usc_bytes")) == len(data), f"usc_bytes {one(meta, 'usc_bytes')} = {len(data)} bytes of words")
    expect(int(one(meta, "usc_dwords")) == len(words), "usc_dwords = the word count")
    expect(one(meta, "sha256") == hashlib.sha256(data).hexdigest(), "sha256 = the words'")
    expect(int(one(meta, "entry_offset")) == 0, "entry_offset 0 (the PDS DOUTU kicks the first group)")

    consts = int(one(meta, "consts_dwords"))
    smp_sh = int(one(meta, "smp_state_sh"))
    tex_sh = int(one(meta, "tex_state_sh"))
    ts = int(one(meta, "texstate_dwords"))
    temps = int(one(meta, "temps"))
    expect(ts == TEXSTATE_DWORDS, f"texstate_dwords {TEXSTATE_DWORDS} (ROGUE_NUM_TEXSTATE_DWORDS)")
    expect(smp_sh + ts <= consts and tex_sh + ts <= consts, "the sampler and texture state slots lie in consts_dwords")
    expect(int(one(meta, "shareds")) <= consts, "shareds (allocated) <= consts_dwords (what the PDS program DMAs)")
    expect(int(one(meta, "coeffs")) == 0 and int(one(meta, "vtxins")) == 0, "no coefficients, no vertex inputs")
    uses = fields(one(meta, "uses"))
    expect(uses.get("sample_shading") == PROPS[2], "sample_shading = is_multisampled")
    expect(uses.get("discard") == "0" and uses.get("usclib") == "0", "no discard, no usclib call")
    outs = [fields(v) for v in meta.get("fs_out", [])]
    fmt = {"1": "PIPE_FORMAT_R32_UINT", "2": "PIPE_FORMAT_R32G32_UINT", "4": "PIPE_FORMAT_R32G32B32A32_UINT"}[PROPS[0]]
    expect(len(outs) == 1 and outs[0].get("res") == str(FRAG_RESULT_DATA0) and outs[0].get("po.start") == "0"
           and outs[0].get("count") == PROPS[0] and outs[0].get("fmt") == fmt,
           "one colour output: FRAG_RESULT_DATA0 in po0.., raw uint dwords")

    os.makedirs(SCRATCH, exist_ok=True)
    binary = os.path.join(SCRATCH, "spm_load.bin")
    open(binary, "wb").write(data)
    listing = usc_dis(binary)
    open(os.path.join(SCRATCH, "spm_load.dis"), "w").write("\n".join(listing) + "\n")
    groups = parse_listing(listing)
    expect(groups, "usc-dis lists instruction groups")
    expect(listing and listing[-1] == f"{len(groups)} instruction groups", "usc-dis decoded to its last line")
    off = 0
    for g_off, size, _, _ in groups:
        expect(g_off == off, f"group at {g_off:#x} follows the one before ({off:#x})")
        off += size
    expect(off == len(data), f"the groups tile the program ({off} of {len(data)} bytes)")
    ends = [g_off for g_off, _, _, g in groups if g["hdr"].get("end") == "1"]
    expect(groups and ends == [groups[-1][0]], f"exactly the last group ends the program (end on {[hex(e) for e in ends]})")
    shared = sorted({int(i) for _, _, _, g in groups for bank, i in g["src"] if bank == "shared"})
    expect(all(s < consts for s in shared), f"every shared register read lies below consts_dwords ({shared})")
    expect(smp_sh in shared and tex_sh in shared, f"the sampler (sh{smp_sh}) and texture (sh{tex_sh}) states are read ({shared})")
    temp = {int(i) for _, _, _, g in groups for bank, i in g["src"] + g["dst"] if bank == "temp"}
    expect(temp and max(temp) < temps, f"every temp lies below temps {temps} ({sorted(temp)})")
    last = groups[-1][3] if groups else {"hdr": {}, "dst": []}
    expect(("special", str(SPECIAL_PIXOUT0)) in last["dst"] and last["hdr"].get("olchk") == "1",
           "the last group writes pixel output 0 (special 32) with the output-register check")

    if bad:
        for b in bad:
            print(f"spm_load check: {b}")
        sys.exit(1)
    print(f"gen_spm_load: spm_load.hex decodes (usc-dis): {len(groups)} instruction groups, {len(data)} bytes; "
          f"temps {temps} (r{min(temp)}..r{max(temp)} used); shared reads at {shared} of consts_dwords {consts} "
          f"(sampler sh[{smp_sh}..{smp_sh + ts}), texture sh[{tex_sh}..{tex_sh + ts})); "
          f"pixout0 written with olchk on the ending group")


def main():
    a = sys.argv[1:]
    is_check = "--check" in a
    a = [x for x in a if x != "--check"]
    if a == ["check"]:
        check()
    elif a[:1] == ["capture"] and len(a) == 2:
        capture(a[1], is_check)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
