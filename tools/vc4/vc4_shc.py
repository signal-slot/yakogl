#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""vc4_shc: what Mesa's vc4 compiler makes of a GLSL ES 1.00 vertex and
fragment shader pair: the QPU words of the fragment, vertex and coordinate
shaders and a description of each one's uniform stream, as JSON.

    tools/vc4/vc4_shc.py <work dir> <vertex.glsl> <fragment.glsl> [--keep <out prefix>] > out.json
    tools/vc4/vc4_shc.py --from <run prefix> > out.json

<work dir> is a build of tools/vc4/drm_shim_ref.py. The pair is compiled by the
driver itself, not by a standalone tool: the program is linked and one
triangle is drawn with it on the drm-shim (drm_shim_ref.py shaders: the draw of
tools/v3d/gl_ref.py, a 64x64 RGBA8 framebuffer, the GL state a new context
has), which makes the driver call vc4_shader_ntq for the three shaders with
the keys of that draw. tools/vc4/README.md says why (Mesa has no entry to its
vc4 compiler outside the gallium context). --keep keeps the run's files;
--from reads the files of an earlier run or shaders form instead of drawing.

The words are the bytes the driver passed to DRM_VC4_CREATE_SHADER_BO (the
shim's record). Which BO is which shader comes from the shader record of the
submit. Mesa's VC4_DEBUG=qpu text must name the same words, or this fails.

The uniform stream of a shader is Mesa's own dump of the words as the draw
writes them (vc4_uniforms.c, switched on by drm_shim_ref.py build): what each
word holds, in stream order. It is checked three ways, and any mismatch
fails: each stream holds the uniforms the shader's final QIR reads (the same
kinds, in the scheduler's order, not the QIR's); the values are the uniform
words of the submit (per shader, in the order fragment, vertex, coordinate:
one BO handle index per word that points into a BO (a texture sample, an
indirect uniform load), then the words); and the streams add
up to uniforms_size. `value` is the word that draw submitted.
"""
import json
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import vc4_submit  # noqa: E402

STAGES = (("fs", "FRAGMENT"), ("vs", "VERTEX"), ("cs", "COORD"))


def die(msg):
    sys.exit("vc4_shc: " + msg)


def describe(prefix):
    """The JSON object of the run whose files start with `prefix`."""
    record = vc4_submit.Record(Path(prefix + ".rec.bin").read_bytes())
    dump, streams = vc4_submit.parse_shader_dump(Path(prefix + ".qpu.txt").read_text())
    # a pair that samples a texture makes the driver submit the clear first,
    # as a job of its own: take the submit with the draw
    drawn = [s for s in record.submits if s.shader_records]
    if len(drawn) != 1 or len(drawn[0].shader_records) != 1:
        die("%s.rec.bin does not hold exactly one shader record" % prefix)
    submit = drawn[0]
    rec = submit.shader_records[0]
    words = submit.uniform_words()
    at, shaders = 0, {}
    for i, (key, stage) in enumerate(STAGES):
        handle = submit.bo_handles[rec["hindex"][i]]
        qpu = record.shader_words(handle)
        # the dump may hold variants the draw did not use: take the one with these words
        text = [d for d in dump if d["stage"] == stage and d["words"] == qpu]
        if not text:
            die("Mesa's QPU dump has no %s shader with the %d recorded words of BO %d" % (stage, len(qpu), handle))
        d = text[-1]
        kinds = [vc4_submit.uniform_kind(u) for u in d["qir_uniforms"]]
        stream = streams.pop(0) if kinds else []
        if sorted(vc4_submit.uniform_kind(w) for _, w in stream) != sorted(kinds):
            die("%s: the uniform dump %s is not the QIR's uniforms %s" % (stage, [w for _, w in stream], kinds))
        samples = sum(1 for k in kinds if vc4_submit.has_bo_handle(k))
        if at + samples + len(stream) > len(words):
            die("%s: the uniform stream runs past uniforms_size" % stage)
        sample_handles = [submit.bo_handles[h] for h in words[at:at + samples]]
        at += samples
        out = []
        for value, what in stream:
            if value != words[at]:
                die("%s uniform %d: the dump says 0x%08x, the submit holds 0x%08x" % (stage, len(out), value, words[at]))
            out.append({"contents": vc4_submit.uniform_kind(what), "value": "0x%08x" % value})
            at += 1
        s = {"bo_handle": handle, "program": "%d/%d" % (d["prog"], d["variant"]),
             "qpu": ["0x%016x" % w for w in qpu], "disasm": d["disasm"],
             "bo_handle_words": samples, "uniform_bo_handles": sample_handles, "uniforms": out}
        if key == "fs":
            s["varyings"] = rec["fs_nvary"]
            s["single_threaded"] = bool(rec["flags"] & 1)
        else:
            _, select, size, _, _ = rec[key]
            s["attribute_select"] = select
            s["attributes_size"] = size
        shaders[key] = s
    if at != len(words) or streams:
        die("the uniform streams are %d words (%d streams left over), uniforms_size is %d words"
            % (at, len(streams), len(words)))
    return {
        "tool": "tools/vc4/vc4_shc.py",
        "draw": "tools/v3d/gl_ref.py triangle with this pair, 64x64 RGBA8, default GL state",
        "shader_record": {"flags": rec["flags"], "attributes": rec["nattr"]},
        "shaders": shaders,
    }


def main():
    a = sys.argv[1:]
    keep = None
    if "--keep" in a:
        i = a.index("--keep")
        if i + 1 >= len(a):
            die("--keep needs a prefix")
        keep = a[i + 1]
        del a[i:i + 2]
    try:
        if len(a) == 2 and a[0] == "--from" and keep is None:
            out = describe(a[1])
        elif len(a) == 3:
            with tempfile.TemporaryDirectory(prefix="vc4_shc.") as tmp:
                prefix = keep or str(Path(tmp) / "pair")
                p = subprocess.run([sys.executable, str(HERE / "drm_shim_ref.py"), "shaders", a[0], a[1], a[2],
                                    prefix], capture_output=True, text=True)
                if p.returncode != 0:
                    sys.stderr.write(p.stderr)
                    die("the draw with this pair failed")
                out = describe(prefix)
        else:
            sys.stderr.write(__doc__)
            return 2
    except (ValueError, OSError) as e:
        die(str(e))
    json.dump(out, sys.stdout, indent=1)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
