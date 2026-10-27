<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Slint replay: the Slint sample's frames through the real library, per GPU

`../slint_gl_rec.c` records what the sample's frames ask of a GL backend, on a
recording stub. This replay runs the same archive and the same frames over the
real `libyakogl.a` (GL core, in-process compiler, back end; Linux OSAL,
nothing rendered), once per GPU the Linux OSAL supports: `powervr`, `vivante`
and `v3d`. The `v3d` run uses the Pi 4 image's 48 MiB carve-out
(`driver/zephyr/lib/board/rpi4/app.conf`; `../../host-replay/README.md`).

## Command

```sh
SLINT_NOSTD_DIR=<no_std slint tree> NOSTD_PATCH_DIR=<femtovg/glow forks> \
  tools/slint/replay/run.sh <out-dir> [powervr] [vivante] [v3d]
```

* `SLINT_NOSTD_DIR`, `NOSTD_PATCH_DIR`: the no_std Slint tree and the
  femtovg/glow forks, recreated as `../../../examples/nostd/README.md` says.
* `REPLAY_FRAMES` (default 300): animation frames after the first, spread over
  the same 22 s as `slint_gl_rec` (idle 0-3 s, acceleration 3-15 s, cruise
  15-22 s), which steps 58 fps (1276 frames).
* `NOTO_SANS_DIR`: only on a host without Noto Sans installed. The UI's font
  is "Noto Sans"; its Light, Regular and SemiBold TTFs in this directory go to
  the Slint compiler as `SLINT_FONT_PATH`.

It needs nightly cargo (`-Z build-std`, so the `rust-src` component), cmake,
cc and python3. The logs are
`<out-dir>/<gpu>.log`, the summary `<out-dir>/report.md`; the exit status is 0
when no GPU found anything. The window is 1024x600 (`main.c`'s); the layout is
`cluster/ui/1024x600`. Selftest of the shared glue:
`../../host-replay/selftest/selftest.sh <out>`.

What a run reports, per frame and in the summary: the GL errors (each with
its entry point), the draws the library refused and the draws that sampled an
incomplete texture (`yakogl_incomplete_texture_draws`: such a draw reads
(0,0,0,1) with no GL error, and the library names only the first incomplete
texture of a process, so the counter is what finds them;
`../../host-replay/README.md`).

## Findings

Run on 2026-10-17, all three GPUs, at `src/` of commit `c854e0ae` (unchanged
by this directory). Inputs: Slint `1007c032` + `slint-nostd.patch` (a fresh
checkout plus the patch compared equal to the tree used, 0 files differ),
femtovg 0.26.0 and glow 0.18.0 with their patches (`../../../examples/nostd/README.md`),
Noto Sans Light/Regular/SemiBold installed on the host. Nightly rustc
1.98.0 (2026-07-01). 301 frames (the first, then REPLAY_FRAMES=300 over
22 s), window 1024x600, shader store 1 MiB. The `v3d` run used the Pi 4's
48 MiB carve-out (`YAKOGL_V3D_CARVEOUT_BYTES=50331648`).

| GPU | GL errors | refused draws | draws sampling an incomplete texture | frames failed | draws per frame |
|---|---|---|---|---|---|
| powervr | none | none | none | 0 of 301 | 475 first, 242-248 after |
| vivante | none | none | none | 0 of 301 | 475 first, 242-248 after |
| v3d | none | none | none | 0 of 301 | 475 first, 242-248 after |

The per-frame results (rc, GL error, refused, incomplete, draws) are identical
on the three GPUs, frame for frame, and the draws per frame are those of the
2026-10-15 run at `5f8fff01`. No GPU logged a GPU-memory refusal, a panic, a
dropped frame or an unexpected line.

Informational lines, not findings:

* **V3D tiled copies** peaked at 887296 of 8388608 bytes (the default budget; the
  Slint sample's `rpi_4b.conf` does not set one).

* **`program <n> has no uniform 'glyphtex'`** (24, every GPU) **and `'tex'`**
  (17, PowerVR only). `glGetUniformLocation` answers -1 without a GL error
  (ES 2.0 2.10.4); FemtoVG asks for optional uniforms. The PowerVR compiler
  drops `tex` from some programs where the GC7000 and V3D ones keep it. These
  frames show no error and no refused draw from it. Follow-up: worth a look
  if a PowerVR image ever draws a textured item wrong.
* **459 unresolved GL names**, the same on every GPU. glow resolves every name
  of its list (desktop ones included) at context creation and panics only on
  a call to an unresolved one; no frame panicked.

**The shader store.** An image lends `CONFIG_YAKOGL_SHADERC_KEEP_BYTES`
(default 1 MiB, `zephyr/Kconfig`), so the replay lends 1 MiB too
(`REPLAY_KEEP_BYTES`; 0 keeps the host build's built-in 512 KiB region). A
second run with `REPLAY_KEEP_BYTES=0` was clean too, on all three GPUs (on
2026-10-15 and again on 2026-10-17): this sample fits in 512 KiB.

The recorded stream is not committed. The archive built from the sample's UI
and the no_std Slint tree is the stream; the command above regenerates it.
A stored trace would hold the texture uploads of the embedded Noto Sans glyph
bitmaps. Noto Sans is OFL-1.1, which `LICENSES/` does not carry. The trace
would also be Slint's compiled output. It would be megabytes per run, too.
The archive is deterministic (a fixed clock), so the command is the record.
