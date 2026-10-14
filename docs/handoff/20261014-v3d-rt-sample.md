# v3d-rt-sample, 2026-10-14, v3d: sample a render target from its render image

## Open work

In the glmark2 item (terrain black #1, AM62P refract #2, the Pi 4 carve-out
#4, #5), after "not yet run on the board.", add:

  Since v3d-rt-sample a colour render target is sampled from its render
  image (the tile list stores RT 0 a second time, in the TMU's layout) and
  its glGenerateMipmap is the TFU's, so refract's and terrain's per-frame
  copies and CPU box filters are gone on the host
  (`docs/online-shaderc/v3d-backend.md`, "Render targets sampled from their
  render image"). Run on the Pi 4 on 2026-10-14
  (`tools/cts/log/board-sessions/pi4-rt-sample.md`): 33 ran / 0 failed /
  0 unsupported, score 188 (183 before), the TFU made terrain's chain.
  CPU time in GL calls per frame went from 711 to 188 ms in refract, from
  442 to 177 ms in terrain, from 228 to 170 ms in shadow, and from 90 to 7 ms
  in desktop blur. refract, terrain and shadow still show 6 FPS. shadow and
  refract still resolve a depth texture on the CPU every frame (option (b),
  not done). Terrain's remaining CPU time is not explained yet.

## Next board session

The host harness (`tools/hostharness/glmark2/run.sh <glmark2> <out> v3d`,
run at this commit on 2026-10-14, 3 frames per scene, 33 ok) shows the
image path for one-level targets only: desktop's 800x600 and terrain's
256x256 targets print "is sampled from its render image (...): no tiled
copy". The Linux OSAL has no TFU (`src/osal/linux.c`, "What is not"), so
there refract's and terrain's glGenerateMipmap stay on the CPU, the CPU
filter moves the stamp, and refract's tiled-copy high-water stays at
19,742,720 bytes (its 7,684,096-byte depth copy and the colour chain copied
again): the TFU chain is tested on the board only. The "render images: ..."
shutdown line is not printed in either place, because glmark2 ends without
tearing the context down. On the board, the glmark2 image's console must
show "the TFU made the 11-level chain of a 1600x1200 render image" in
refract (the 10-level 800x600 one in terrain), no "tiled copy" line naming
a colour render target, and 33 / 0 / 0. Terrain's remaining CPU cost is
still unexplained: on the host its tiled-copy high-water climbs to
12,433,408 bytes before its first frame ends, and which textures those
copies are (uploads, or targets sampled before their first render) is not
read from the console.
