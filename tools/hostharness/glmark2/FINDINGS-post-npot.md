# glmark2 host harness after `npot` merged: NOT RUN (no glmark2 source)

**Status: the harness was not run.** This note has no measured "after"
numbers. Fetching glmark2 needs the network, and this worker refused every
network command (`git clone`, `curl`). No glmark2 checkout exists on the
machine. Nothing below is measured except what is marked "checked".

Re-run it on any machine that can fetch glmark2. The commands are at the
end. Another item (or a person) can then fill the "after" columns.

## 1. Commit and commands

* Tree: `fdff1147b151a4a240126371a06f45f69a310601` (ccfarm/integration tip).
  The npot commits are in it: `2673197d` (core), `77693ff4`, `be938366`
  (merge fixes), per `git log`.
* Checked here: `tools/check.sh --quick` -> `99 ok, 0 failed, 0 gap(s)`.
* Not run (no source): the commands for the three GPUs. They are in
  "How to re-run" below.

## 2. glGenerateMipmap 0x502 on terrain and refract

Measured: not available. Checked in the code, which is the same for all GPUs:

* `GL_OES_texture_npot` is claimed when `REPORT_ES2` is on:
  `src/gl/state.rs:186` (`NPOT_EXTENSION = crate::REPORT_ES2`), listed at
  `src/gl/state.rs:147`.
* The error is raised only when `NPOT_RESTRICTED` holds
  (`src/gl/texture.rs:3244`). It is defined at `src/gl/texture.rs:855` as
  `REPORT_ES2 && !NPOT_EXTENSION`, which is always `false`. A unit test pins
  it (`src/gl/state.rs:1443`).
* `glGenerateMipmap` is in the GL core, not in a back end, so no back end can
  skip the claim. The harness builds with `YAKOGL_REPORT_ES2=ON`
  (`run.sh`), so the claim is on.
* Other 0x502 causes in `glGenerateMipmap` (depth texture,
  `src/gl/texture.rs:3252`; bad target or no texture) do not apply to
  terrain and refract's colour textures.

So the expectation is: gone on powervr, vivante and v3d. This is code reading,
not a run, so it is not proof for glmark2's real stream.

Diff table (before = FINDINGS.md / FINDINGS-v3d.md; after = **not measured**):

| entry | GPU | line | before | after |
|---|---|---|---|---|
| 22 terrain | powervr | `GL error 0x502 in glGenerateMipmap` | 11 | not run (expect 0) |
| 22 terrain | vivante | `GL error 0x502 in glGenerateMipmap` | 11 | not run (expect 0) |
| 22 terrain | v3d | `GL error 0x502 in glGenerateMipmap` | 11 | not run (expect 0) |
| 24 refract | powervr | `GL error 0x502 in glGenerateMipmap` | 3 | not run (expect 0) |
| 24 refract | vivante | `GL error 0x502 in glGenerateMipmap` | 3 | not run (expect 0) |
| 24 refract | v3d | `GL error 0x502 in glGenerateMipmap` | 3 | not run (expect 0) |

New GL errors, refused draws and incomplete-texture lines: not measured.
Things to look at in the re-run, because npot changes behaviour there:

* Incomplete-texture lines for terrain/refract should go (the NPOT mip chain
  is now complete). `src/gl/draw.rs:2349` only flags NPOT when
  `NPOT_RESTRICTED`.
* V3D: `grow_npot_chain` (`src/v3d/texture.rs:52`, `:124`) lays out a chain
  later, so terrain and refract may use more GPU memory and tiled-copy
  space than before. Compare the "GPU memory" table with FINDINGS-v3d.md.
* PowerVR keeps one level of an NPOT image (`src/gl/draw.rs:2289`); item
  `pvr-npot-mips` changes that, so run before and after it if both merged.
* GC7000: the hardware NPOT bit is item `gc-npot-bit`'s work.

## 3. Follow-up lines

Not measured. Before (FINDINGS.md:73,75 and FINDINGS-v3d.md:134,140):

| entry | GPU | line | before | after |
|---|---|---|---|---|
| 23 shadow | powervr | `the backend did not resolve its depth (-16)` | 3 | not run |
| 24 refract | powervr | `the backend did not resolve its depth (-16)` | 3 | not run |
| 23 shadow | v3d | `a stale tiled copy of 7684096 bytes given back before the job opened` | 1 | not run |
| 24 refract | v3d | `a stale tiled copy of 7684096 bytes given back before the job opened` | 1 | not run |

Item `pvr-depth-resolve` is still `[ready]`, so the PowerVR line should still
be there. `v3d-rt-sample` (running) targets the V3D line.

## 4. V3D GPU-memory peak under the 96 MiB carve-out

Not measured. FINDINGS-v3d.md (before npot) lists the largest high-waters as
refract 55,527,424 bytes and terrain next (lines ~161-181 there). The re-run
prints the per-entry table in its generated GPU-memory section.

## How to re-run

Get glmark2 (needs network), then run with `GM2_WRITE=0` so no tracked file
is rewritten. The table is then not written either, so use `report.py` by
hand to send its output to a scratch file:

```sh
git clone https://github.com/glmark2/glmark2 "$GLMARK2_SRC"
git -C "$GLMARK2_SRC" checkout 22c527cb0556f3a1ac4445aaa52cc532760928d5
GM2_WRITE=0 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-post-npot
```

`run.sh` runs all three GPUs (powervr, vivante, v3d with the 96 MiB
carve-out set by `YAKOGL_V3D_CARVEOUT_BYTES`, default from run.sh). The logs
are under `build/glmark2-post-npot/`. For the findings tables, copy
`report.py` to a scratch dir and call it with `--findings <scratch>.md`, as
`run.sh` lines 218-229 do. Do not use `GM2_WRITE=1`: it rewrites
`FINDINGS.md`, `FINDINGS-v3d.md` and `tools/shaders/glmark2/`.
