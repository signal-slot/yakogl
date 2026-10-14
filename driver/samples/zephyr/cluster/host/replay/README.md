# Cluster replay: the cluster's frames through the real library, per GPU

`glcount/` runs the image's Rust library over a stub GL that counts calls.
This replay runs the same library, with the same frame sequence, over the real
`libyakogl.a` (GL core, in-process compiler, back end; Linux OSAL, nothing
rendered), once per GPU the Linux OSAL supports: `powervr`, `vivante` and
`v3d`. Each runs at its board's panel size: 1024x600 (AM62P), 1280x800
(i.MX8MP), 1920x720 (Pi 4). The `v3d` run uses the Pi 4 image's 64 MiB
carve-out and 16 MiB tiled-copy budget (`../../boards/*.conf`;
`../../../host-replay/README.md`).

## Command

```sh
AGL_SLINT_CLUSTER_DIR=<clone> SLINT_NOSTD_DIR=<no_std slint tree> \
NOSTD_PATCH_DIR=<femtovg/glow forks> \
  driver/samples/zephyr/cluster/host/replay/run.sh <out-dir> [powervr] [vivante] [v3d]
```

* `AGL_SLINT_CLUSTER_DIR`: <https://github.com/signal-slot/agl-slint-cluster>
  (the `ui/` the image compiles; `../../CMakeLists.txt`).
* `SLINT_NOSTD_DIR`, `NOSTD_PATCH_DIR`: the no_std Slint tree and the
  femtovg/glow forks, recreated as `../../../slint/nostd/README.md` says.
* `NOTO_SANS_DIR` (default `/usr/share/fonts/noto`), `REPLAY_FRAMES`
  (default 20 frames after the check's two).

It needs nightly cargo (`-Z build-std`, so the `rust-src` component), cmake,
cc and python3. The logs are
`<out-dir>/<gpu>.log`, the summary `<out-dir>/report.md`; the exit status is 0
when no GPU found anything.

The frames are glcount's: Slint clock 0 and 1000 ms (400 telemetry ticks
between them), then 20 frames 200 ms apart with 4 ticks each. The window is
`main.c`'s, the board's panel (`REPLAY_W`/`REPLAY_H` override it). Selftest of
the shared glue:
`../../../host-replay/selftest/selftest.sh <out>`.

## Findings

Run on 2026-10-15, all three GPUs, at `src/` of commit `5f8fff01` (unchanged
by this directory). Inputs: agl-slint-cluster `a8a03d08` (its default branch),
Slint `1007c032` + `slint-nostd.patch`, femtovg 0.26.0 and glow 0.18.0 with
their patches (`../../../slint/nostd/README.md`; all three applied cleanly),
Noto Sans from notofonts.github.io. Nightly rustc 1.101.0 (2026-10-11).
22 frames (REPLAY_FRAMES=20 after the check's two), shader store 1 MiB. Each
GPU used its board's panel: powervr 1024x600, vivante 1280x800, v3d 1920x720
with the Pi 4's 64 MiB carve-out and 16 MiB tiled-copy budget.

| GPU | GL errors | refused draws | incomplete textures | frames failed | draws per frame |
|---|---|---|---|---|---|
| powervr | none | none | none | 0 of 22 | 297 first, 165-177 after |
| vivante | none | none | none | 0 of 22 | 297 first, 165-177 after |
| v3d | none | none | none | 0 of 22 | 297 first, 165-177 after |

No GPU logged a GPU-memory refusal, a panic, a dropped frame or an
unexpected line. The replay does not call `cluster_board_mem` (the image's
memory-stage line in `main.c`/`rust/src/board.rs`); it draws no GL.

Informational lines, not findings:

* **V3D tiled copies** peaked at 12174848 of the 16777216-byte budget (73 %)
  at 1920x720. That leaves 4.4 MiB of headroom. A UI that adds sampled
  render targets at that size would reach it first.
* **`program <n> has no uniform 'glyphtex'`** (24, every GPU) **and `'tex'`**
  (17, PowerVR only). `glGetUniformLocation` answers -1 without a GL error
  (ES 2.0 2.10.4); FemtoVG asks for optional uniforms. The PowerVR compiler
  drops `tex` from some programs where the GC7000 and V3D ones keep it. These
  frames show no error and no refused draw from it. Follow-up: worth a look
  if a PowerVR image ever draws a textured item wrong.
* **459 unresolved GL names**, the same on every GPU. glow resolves every name
  of its list (desktop ones included) at context creation and panics only on
  a call to an unresolved one; no frame panicked.

**The shader store matters on the PowerVR.** An image lends
`CONFIG_YAKOGL_SHADERC_KEEP_BYTES` (default 1 MiB, `zephyr/Kconfig`), so the
replay lends 1 MiB too (`REPLAY_KEEP_BYTES`; 0 keeps the host build's built-in
512 KiB region). With `REPLAY_KEEP_BYTES=0` the PowerVR run refuses 145 draws:
5 in the first frame, 10 in frames 2-8, then 5 per frame. The log says
"no room for the blend key ... variant (online shaderc: keep full on this
fragment shader: 7850 bytes free in its largest run, 514304 of 524288 bytes
in use)". The GC7000 and V3D runs stay clean at 512 KiB, as does the Slint
sample on all three. Follow-up: a PowerVR cluster image needs more than
512 KiB of store. The 1 MiB default has about 2x the headroom; no board file
sets it lower today.

The recorded stream is not committed. The archive built from the sample's UI
(agl-slint-cluster, MIT, outside this repository) and the no_std Slint tree is
the stream; the command above regenerates it. A stored trace would hold the
Noto Sans glyph uploads (OFL-1.1, not in `LICENSES/`) and would be megabytes
per run. The archive is deterministic (a fixed clock).
