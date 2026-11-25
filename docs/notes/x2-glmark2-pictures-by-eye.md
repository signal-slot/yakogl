<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Armadillo-X2 (i.MX8MP, GC7000UL): the 33 glmark2-es2 pictures, looked at by eye

This was to be the X2's contribution to "Pictures by eye" (ROADMAP stage 2
"Left for the criterion", issue #17): look at the 33 glmark2-es2 scenes at
800x600 on the X2 panel through the farm's HDMI KVM (`kvm-armadillo-x2`,
observation only), from one origin/main commit, and settle the
`[texture] texture-filter=mipmap` picture that changed at `0d21ab47`.
It did not happen. This note is the stop record: what held, what stopped it,
the 33 rows with their verdicts, and what unblocks a rerun. The measured-side
stop of the same board and the same cause is `docs/notes/x2-glmark2-slow-scenes.md`
("Stop, 2026-11-24", "Session A: stop, 2026-11-25").

No board address, host name, UART path, USB serial or credential is written
in this note, and none may go into a record of a future session: they are the
environment's.

## Which board this is

The Armadillo-X2 has the same SoC (i.MX8MP) and GPU (GC7000UL) as the Verdin
iMX8MP, but it is not the stage-2 board; the stage-2 criterion board stays the
Verdin. A verdict written here is a same-GPU reading, not a row of the
stage-2 criterion, and never a `tools/cts/RESULTS.md` row (the X2 runs are
kexec-dev development runs, `docs/notes/x2-glmark2-slow-scenes.md`).

## Stop, 2026-11-25 (plan #411, item x2-eye-stop)

**No scene was looked at. No image was built for the board, nothing was
staged or copied to it, no jump was made, no KVM view was opened; the board
was left as found, up on Linux.** The 33 verdict cells below are stop cells.

Cause — the same stop that held the measurement sessions:

- Three leased task machines found the board reachable over ssh on
  2026-11-25 (up about 2 days 7 h, not rebooted): runlevel `default`,
  `kexec_loaded 0`, `kexec_crash_loaded 0`.
- `podman ps -a` on the board lists the container `slint-launcher`
  (image `at-debian-image`) `Up 2 days`, with `set_autostart yes` in
  `/etc/atmark/containers/slint-launcher.conf`: it is back at every boot,
  and every run that ends in an SoC reset brings it back.
- `tools/board.sh x2 run` refuses a jump while any container runs — its
  readiness check (`x2_kexec_ready`, "nobody's application running",
  `tools/board.sh:2518-2526` of the current tree; the session quoted its own
  checkout's range) reads `something is running (containers 1)`
  and exits rc 3: a crash-route kexec jump would end the container without
  a shutdown. A by-eye run is exactly a sequence of `board.sh x2 run`
  jumps (one per scene), so the 33-scene look could not even start: the
  first scene would fail where the 2026-11-24 terrain run failed.
- Stopping the container or turning its autostart off changes the board's
  rootfs and services. This plan's rules say leave the board as found and
  persist nothing, so that is the board owner's decision, and the owner did
  not answer on 2026-11-25. The container stands; the look did not happen.

The same stop happened on 2026-11-24, at the first `board.sh x2 run` of
`tools/cts/x2-glmark2-scenes.sh --commit 72972ba4 records/x2/20261124-1035-before-72972ba4 terrain shadow refract`
(record `records/x2/20261124-1035-before-72972ba4`, `refused=something is
running (containers 1)`, `refused_rc=3`), quoted in
`docs/notes/x2-glmark2-slow-scenes.md`, "Stop, 2026-11-24". That run built
its first image and stopped in the readiness read before staging. This item
did not reach an image build either.

**The commit the attempt would have used.** `tools/cts/x2-glmark2-scenes.sh`
names no fixed SHA: it takes `--commit <rev>` and refuses a commit that is
not an ancestor of `origin/main` (`tools/cts/x2-glmark2-scenes.sh:29-32`,
`:108-119`). `docs/notes/gc-tiled-wire.md` ("Session A: the run sheet", step
1) is the rule: one SHA per session, that day's `git rev-parse origin/main`,
carrying W1-W5. The 2026-11-25 session task read `origin/main` as
`3c3fe33b` — checked here: it carries the `gc_tiled_rt` knob and `9cdeff6d`
(W1 to W5) is its ancestor — and its same-day re-checks read `d4f95c29` and
`3454235c` (refs moving between tasks). The by-eye look would have used its
session's single origin/main commit in the same way, images built from
origin/main commits only; which eight hex digits it prints is that session's
`<sha8>`, and no run ever reached the point of naming one.

## The 33 scenes

The scene list is `tools/cts/glmark2-pictures.txt` (33 scenes, the `imx8mp`
column); the expected pictures are the content of the upstream glmark2
reference picture (a Mesa llvmpipe run, `tools/cts/glmark2-host-ref.sh`, the
judge of `docs/notes/imx8mp-glmark2-pictures-by-eye.md`) read against the
value pattern of the `imx8mp` column (probes, mean; probes are RGB565 widened
to 8 bits, alpha 00). They say what a correct picture should look like; no
X2 picture was seen, so no row was judged. A rerun fills the verdict column
with `right`, `wrong` or `seen without a value`, as that note does.

| scene | expected picture (short) | verdict |
|---|---|---|
| [build] use-vbo=false | the house model, built per frame from separate draws; grey lit centred on black (probe c `a0a0a0`, corners black) | not looked at (stop) |
| [build] use-vbo=true | the same house built from vertex arrays (VBO); picture as the row above (c `a0a4a0`) | not looked at (stop) |
| [texture] texture-filter=nearest | a textured quad centred, crisp texel edges (nearest filtering); faint texture corner at tl (`181c18`) | not looked at (stop) |
| [texture] texture-filter=linear | the same quad bilinear-filtered: texels smoothed | not looked at (stop) |
| [texture] texture-filter=mipmap | the same quad mipmapped (a mip chain per pixel); the scene with the open picture change — see the note under the table | not looked at (stop) |
| [shading] shading=gouraud | the bunny lit with Gouraud (vertex-interpolated) shading, blue-dominant (c `0000c8`, mean 0 0 17), corners dark | not looked at (stop) |
| [shading] shading=blinn-phong-inf | the same bunny with per-fragment Blinn-Phong highlights (mean 5 5 18) | not looked at (stop) |
| [shading] shading=phong | the same bunny with Phong highlights, brighter blue (mean 1 1 28) | not looked at (stop) |
| [shading] shading=cel | the same bunny cel-shaded in flat green bands (mean 0 25 0) | not looked at (stop) |
| [bump] bump-render=high-poly | a grey bumpy high-poly model centred, bump-mapped light play, corners dark (c `a0a0a0`) | not looked at (stop) |
| [bump] bump-render=normals | the same with the normal-map variant (c `a8a8a8`) | not looked at (stop) |
| [bump] bump-render=height | the same with the height-map variant (c `989898`) | not looked at (stop) |
| [effect2d] kernel=0,1,0;1,-4,1;0,1,0; | the texture sharpened: a near-black frame, faint texture at the centre (mean 0 0 1) | not looked at (stop) |
| [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1; | the texture box-blurred: a near-white bright frame, all probes bright (mean 231 233 221) | not looked at (stop) |
| [pulsar] light=false:quads=5:texture=false | five coloured quads over a black frame (c `700070`, tl `580410`, br `781848`; reds and magentas) | not looked at (stop) |
| [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4 | a bright desktop with four blurred windows over it (near-white centre, mean 204 210 189) | not looked at (stop) |
| [desktop] effect=shadow:windows=4 | a bright desktop with four Tux windows carrying drop shadows (mean 197 203 182) | not looked at (stop) |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map | an animation of 200 columns of bluish quads, half re-uploaded per frame via map (mean 45 59 67); probe c moves frame to frame | not looked at (stop) |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata | the same grid updated via BufferSubData (c `b0d8f0`) | not looked at (stop) |
| [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map | the same grid, interleaved columns | not looked at (stop) |
| [ideas] speed=duration | a mostly dark frame with a small coloured speedometer-style dial centred (c `282038`, mean 5 4 7) | not looked at (stop) |
| [jellyfish] <default> | a jellyfish with animated tentacles over a blue background (tl and tr `508cf8`, mean 69 102 191) | not looked at (stop) |
| [terrain] <default> | green hills under a low flying camera, all five probes green-dominant (mean 119 129 45); "green hills", as looked at on the AM62P panel at `bc43ea09` | not looked at (stop) |
| [shadow] <default> | a grey floor with a moving light and its cast shadow, grey everywhere (mean 34 34 34), corners dark | not looked at (stop) |
| [refract] <default> | a glassy bunny refracting a checker texture over a dark background (c `706cd0`, mean 22 18 29); "the bunny", as looked at on the AM62P panel at `bc43ea09` | not looked at (stop) |
| [conditionals] fragment-steps=0:vertex-steps=0 | a lit field of white blocks with a black centre area (tr `f8fcf8`, br `f8f8f8`, mean 56 56 56) | not looked at (stop) |
| [conditionals] fragment-steps=5:vertex-steps=0 | the same field, five fragment-side conditional steps | not looked at (stop) |
| [conditionals] fragment-steps=0:vertex-steps=5 | the same field, five vertex-side conditional steps | not looked at (stop) |
| [function] fragment-complexity=low:fragment-steps=5 | grey blocks grading brighter to the lower right (tr `282c28`, bl `606060`, br `808480`), black centre | not looked at (stop) |
| [function] fragment-complexity=medium:fragment-steps=5 | the same with the more complex fragment function | not looked at (stop) |
| [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5 | the same grey-block field, the loop-variant shader (no fragment loop) | not looked at (stop) |
| [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5 | the same, non-uniform fragment loop | not looked at (stop) |
| [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5 | the same, uniform fragment loop | not looked at (stop) |

### The mipmap row, and why it stayed open

`[texture] texture-filter=mipmap` is the one scene whose i.MX8MP picture is
known to have changed without being explained. Per `tools/cts/RESULTS.md`
(i.MX8MP notes): probe tl `181c18` at `a8fdc9f0` and `4ba42a28` became
`000000` from `0d21ab47` on (bisected on the board at `c5f10ea9`); the only
library change recorded for `0d21ab47` is `ds_cache::resolve_depth`, which
this scene never runs (`docs/notes/gc-depth-resolve.md`), so the mechanism is
open. `0d21ab47` is not an object in this workspace (`git cat-file -t
0d21ab47` fails), so its content could not be read from this farm, and this
note adds nothing to the record above it. Whether the old picture (a lit
texture corner at tl) or the new one (black there) is the right one is not
known: no camera on the Verdin panel, and the X2 look — which could have
answered it by eye — stopped. See `tools/cts/STATUS.md` and
`docs/notes/stage2-bench-gap.md` ("Left"); `dEQP-GLES2 texture.mipmap.*` read
98 pass, 0 fail, 14 quality warnings at `c5f10ea9`, which does not decide
the picture either.

## Where this leaves the criterion items

- ROADMAP stage 2 "Left for the criterion", "Pictures by eye" (issue #17):
  the i.MX8MP part stays open. The `imx8mp` column of
  `tools/cts/glmark2-pictures.txt` stays a regression baseline nobody has
  looked at, as `docs/notes/stage2-bench-gap.md` says.
- `docs/notes/imx8mp-glmark2-pictures-by-eye.md` (the Verdin by-eye note,
  issue #1196): untouched by this stop, still its skeleton; its dump and
  camera routes are the Verdin's own and remain unfilled.
- A future X2 look, if it happens, writes its verdicts here (this file is
  its record sheet) and never into the Verdin note or into
  `tools/cts/RESULTS.md`.

## What unblocks it

1. The board owner decides the container. Stopping `slint-launcher` before
   each run is not enough by itself (every run ends with an SoC reset and
   the launcher autostarts at boot), so a session needs the owner's choice:
   stop it before every jump for the session, or turn its autostart off for
   the session (a change to the board's rootfs, which the current plan rules
   reserve to the owner).
2. Then, from a leased worker (`target:x2`; the panel view through
   `kvm-armadillo-x2`, observation only; the console read-only): the run
   sheet order of `docs/notes/gc-tiled-wire.md` step 2 first — reachability
   and `tools/board.sh x2 status`, which must read `containers 0`.
3. Then one origin/main commit for the whole session (images built from
   origin/main commits only), one glmark2 scene per `board.sh x2 run`, each
   picture looked at on the panel through the KVM and judged by content
   against the llvmpipe reference of `tools/cts/glmark2-host-ref.sh`;
   records under `records/x2/<YYYYMMDD-HHMM>-<name>-<sha8>/`, redacted as
   above.
4. The mipmap row can then be judged — that is the one row with a known open
   question, and a by-eye verdict on the X2 (same GPU) is the cheap route to
   "which picture is right" while the Verdin has no camera.
