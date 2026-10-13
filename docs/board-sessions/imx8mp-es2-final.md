<!-- SPDX-License-Identifier: Apache-2.0 -->
# i.MX8MP dEQP-GLES2 whole tree at origin/main `cd738b1a` (imx8mp-es2-final)

Plan #20 item `imx8mp-es2-final`, roadmap stage 1 (zero dEQP-GLES2 failures).
Date: 2026-10-14. Board: Verdin i.MX8M Plus (GC7000), the leased i.MX8MP farm
resource, `BOARD_BOOT=uboot-mmc`. The board has no UART in this setup: every result
is the SNVS record `tools/board.sh` reads after each cycle. No board address,
credential or UART path is written here.

## Summary

| what | result |
|---|---|
| Commit | origin/main `cd738b1a` (`cd738b1ae7cf99166a6bdd86cf24dac04c4c4328`, "ccfarm: merge ccfarm/01a13623-0a20-7578-bba9-a6ab4290251a (#533)"), after `git fetch origin` |
| Image | `deqp-cd738b1a/zephyr/zephyr.bin`, sha256 prefix `5994ba33c5f9e596`; image build `PVR_SAMPLE_TABLE=c95547a1 PVR_SAMPLE_TREE=cd738b1a`; Zephyr `v4.4.0-9848-gaf17c0c8aa2a` |
| Build | `tools/cts/imx8mp-deqp-build.sh deqp-cd738b1a 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt` rc=0, 29 table rows, `CONFIG_DEQP_REPORT_ES2=y`, conformance-image check passed; stage-1 libraries from the GLES2 tree `deqp-stage1-imx8mp` |
| Run 1 | `tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt records/imx8mp/deqp-cd738b1a`: all 29 entries + 12 `DEQP_FAILS` name pages, every row `rc=0 verdict=done`, GPU resets absent/0 |
| Run 2 | same image, same command into `records/imx8mp/deqp-cd738b1a-run2`: identical per case |
| Failures | **46** in both runs, all in entry 21 (`fbo`), every one a `GL_BGRA` renderbuffer case. **A regression against `c345ab74` (0 failures)**; cause below |

TALLY line of run 1, verbatim:

```
TALLY (29 entries): begun 17371 passed 17272 failed 46 notsup 34 other 19 noresult 0
```

`board.sh` printed `tree rc=0` and `tally rc=0` (the `tally` line is the same string).

**Run 2**, the same image again into `records/imx8mp/deqp-cd738b1a-run2`: every
row `rc=0 verdict=done` again, TALLY verbatim:

```
TALLY (29 entries): begun 17371 passed 17272 failed 46 notsup 34 other 19 noresult 0
```

The two runs agree per case: `results.tsv` of run 2 is byte-identical to run 1's
(same counts and the same 46 failing names), and all 41 row lines (29 entries + 12
name pages) carry the same SNVS record words in both runs. The failures are
deterministic, not a flake. (`board.sh` labels run 2 `build 41cd78a4-dirty` from
e02 on because this file was being edited in the workspace meanwhile; the image,
sha256 prefix `5994ba33c5f9e596`, is the same file.)

The image was built from this item's workspace, whose HEAD was `cd738b1a` with a
clean tree (`board.sh` names the build `41cd78a4`, the workspace HEAD after this
record's first commit, a docs-only commit on top of `cd738b1a`; the image's own
`PVR_SAMPLE_TREE` is `cd738b1a`). `/var/yakogl` on the board was empty before the
run: no stale staged image this time.

## Failing cases (46, entry 21)

All are in `tools/cts/gles2-mustpass.txt`. Decoded from the `DEQP_FAILS` pages
`e21-p0` .. `e21-p7`:

- `dEQP-GLES2.functional.fbo.completeness.renderable.renderbuffer.` +
  `color0.bgra`, `depth.bgra`, `stencil.bgra`
- `dEQP-GLES2.functional.fbo.render.` +
  - `color.blend_npot_rbo_bgra`, `color.blend_npot_rbo_bgra_depth_component16`,
    `color.blend_rbo_bgra`, `color.blend_rbo_bgra_depth_component16`
  - `color_clear.rbo_bgra`, `color_clear.rbo_bgra_depth_component16`,
    `color_clear.rbo_bgra_depth_component16_stencil_index8`,
    `color_clear.rbo_bgra_stencil_index8`
  - `depth.npot_rbo_bgra_depth_component16`, `depth.rbo_bgra_depth_component16`
  - `recreate_colorbuffer.no_rebind_rbo_bgra`,
    `recreate_colorbuffer.no_rebind_rbo_bgra_depth_component16`,
    `recreate_colorbuffer.no_rebind_rbo_bgra_depth_component16_stencil_index8`,
    `recreate_colorbuffer.no_rebind_rbo_bgra_stencil_index8`,
    `recreate_colorbuffer.rebind_rbo_bgra`,
    `recreate_colorbuffer.rebind_rbo_bgra_depth_component16`,
    `recreate_colorbuffer.rebind_rbo_bgra_depth_component16_stencil_index8`,
    `recreate_colorbuffer.rebind_rbo_bgra_stencil_index8`
  - `recreate_depthbuffer.no_rebind_rbo_bgra_depth_component16`,
    `recreate_depthbuffer.no_rebind_rbo_bgra_depth_component16_stencil_index8`,
    `recreate_depthbuffer.rebind_rbo_bgra_depth_component16`,
    `recreate_depthbuffer.rebind_rbo_bgra_depth_component16_stencil_index8`
  - `recreate_stencilbuffer.no_rebind_rbo_bgra_depth_component16_stencil_index8`,
    `recreate_stencilbuffer.no_rebind_rbo_bgra_stencil_index8`,
    `recreate_stencilbuffer.rebind_rbo_bgra_depth_component16_stencil_index8`,
    `recreate_stencilbuffer.rebind_rbo_bgra_stencil_index8`
  - `resize.rbo_bgra`, `resize.rbo_bgra_depth_component16`,
    `resize.rbo_bgra_depth_component16_stencil_index8`, `resize.rbo_bgra_stencil_index8`
  - `shared_colorbuffer.rbo_bgra`, `shared_colorbuffer.rbo_bgra_depth_component16`,
    `shared_colorbuffer.rbo_bgra_depth_component16_stencil_index8`,
    `shared_colorbuffer.rbo_bgra_stencil_index8`, `shared_colorbuffer_clear.rbo_bgra`
  - `shared_depthbuffer.rbo_bgra_depth_component16`,
    `shared_depthbuffer.rbo_bgra_depth_component16_stencil_index8`
  - `stencil.npot_rbo_bgra_depth_component16_stencil_index8`,
    `stencil.npot_rbo_bgra_stencil_index8`,
    `stencil.rbo_bgra_depth_component16_stencil_index8`, `stencil.rbo_bgra_stencil_index8`
  - `stencil_clear.rbo_bgra_depth_component16_stencil_index8`,
    `stencil_clear.rbo_bgra_stencil_index8`

### Cause (from the source; no fix in this item)

`8bbf9ba2` ("report-es2: buffer and framebuffer entry points refuse ES 3.0-only
values (es2-error-rules rows 1-3, 34-36)") is the only commit between `c345ab74`
and `cd738b1a` that changes the renderbuffer format rules. It turned
`renderbuffer_format_advertised` (`src/gl/framebuffer.rs`) into an allowlist under
`report-es2`. The list names `GL_BGRA8_EXT` (`0x93A1`) but not the unsized
`GL_BGRA_EXT` (`0x80E1`), so `glRenderbufferStorage(GL_BGRA_EXT, ...)` now falls
to `_ => false` and is refused. `is_renderbuffer_color_format` still takes
`GL_BGRA_EXT`, and before `8bbf9ba2` the function returned true for every format
except `GL_DEPTH_COMPONENT24`.

dEQP-GLES2 creates these renderbuffers with the unsized `GL_BGRA`
(`es2fFboRenderTest.cpp:2225`, `{GL_RENDERBUFFER, GL_BGRA}`) and only checks
`GL_EXT_texture_format_BGRA8888`, which this build advertises
(`es2fFboRenderTest.cpp:461-464`). So the cases run and fail instead of being
NotSupported. Of the 102 `fbo.*bgra*` names in the mustpass list, the other 56
still pass: the 53 texture ones (`tex2d_bgra` and friends), which do not take this
path, and `completeness.renderable.renderbuffer.{color0,depth,stencil}.bgra8_ext`,
which use the sized name the allowlist does have.

The fix belongs in `src/gl/framebuffer.rs` (outside this item): either add
`GL_BGRA_EXT` to the `GL_BGRA8_EXT` arm, or decide (es2-error-rules row 36) that
neither BGRA name is renderbuffer storage. Only the first keeps these 46 cases
passing. The change is in the GL core, so the AM62P and Pi 4 `report-es2` trees
built at or after `8bbf9ba2` should show the same 46 names unless their back end
refuses BGRA renderbuffers anyway. This was not checked on those boards here.

## Mustpass totals (the 17,165 list)

As in `imx8mp-es2-final-r2.md`: the tree runs 17,371 names, 206 of them outside
`tools/cts/gles2-mustpass.txt`. The 46 failures and the 19 QualityWarning names are
all in the list. The 34 NotSupported results are counts only (entries 22 and 28),
so 0 to 4 of them may be among the 206 extra names and the totals are a range:

| | Pass | Fail | NotSupported | QualityWarning |
|---|---:|---:|---:|---:|
| 17,165 list | 17,066 to 17,070 | 46 | 30 to 34 | 19 |

Both ends sum to 17,165. The whole-run counts are 17,272 pass / 46 fail / 34
NotSupported / 19 QualityWarning.

## Not Pass (other than the failures)

Unchanged from `c345ab74`:

- QualityWarning 19: entry 14, 14 `texture.mipmap.cube.generate.*` cases; entry 27,
  5 `rasterization.interpolation.{basic,projected}.line*` cases (names as in
  `imx8mp-es2-final-r2.md`).
- NotSupported 34, counts only: entry 22 (`negative_api`) 9, entry 28 25.

## Per entry against `c345ab74` (`imx8mp-es2-final-r2.md`)

`diff tools/cts/imx8mp-full-tree-29460f96.results.tsv records/imx8mp/deqp-cd738b1a/results.tsv`
(the baseline file r2 matched byte for byte) shows **one** changed line, entry 21:

```
< 21	dEQP-GLES2.functional.fbo.*	711	711	0	0	0	0	-
> 21	dEQP-GLES2.functional.fbo.*	711	665	46	0	0	0	<the 46 names above>
```

| entry | begun | passed | failed | notsup | other | vs c345ab74 |
|---|---:|---:|---:|---:|---:|---|
| 0 info, prerequisite | 9 | 9 | 0 | 0 | 0 | same |
| 1 color_clear | 19 | 19 | 0 | 0 | 0 | same |
| 2 shaders (20 groups) | 1512 | 1512 | 0 | 0 | 0 | same |
| 3 matrix, invalid_implicit_conversions | 994 | 994 | 0 | 0 | 0 | same |
| 4 swizzles, conversions | 1172 | 1172 | 0 | 0 | 0 | same |
| 5 indexing, preprocessor | 1100 | 1100 | 0 | 0 | 0 | same |
| 6 loops | 625 | 625 | 0 | 0 | 0 | same |
| 7 random | 1900 | 1900 | 0 | 0 | 0 | same |
| 8 operator.binary_operator | 1318 | 1318 | 0 | 0 | 0 | same |
| 9 operator (other 10 groups) | 1631 | 1631 | 0 | 0 | 0 | same |
| 10 texture.completeness | 40 | 40 | 0 | 0 | 0 | same |
| 11 texture.filtering.2d | 136 | 136 | 0 | 0 | 0 | same |
| 12 texture.filtering.cube | 112 | 112 | 0 | 0 | 0 | same |
| 13 texture.format | 36 | 36 | 0 | 0 | 0 | same |
| 14 texture.mipmap | 112 | 98 | 0 | 0 | 14 | same |
| 15 texture.size | 80 | 80 | 0 | 0 | 0 | same |
| 16 texture.specification | 192 | 192 | 0 | 0 | 0 | same |
| 17 texture.vertex | 90 | 90 | 0 | 0 | 0 | same |
| 18 texture.wrap | 58 | 58 | 0 | 0 | 0 | same |
| 19 fragment_ops.blend | 1060 | 1060 | 0 | 0 | 0 | same |
| 20 fragment_ops (rest) | 863 | 863 | 0 | 0 | 0 | same |
| 21 fbo | 711 | **665** | **46** | 0 | 0 | **+46 failed** |
| 22 negative_api | 246 | 237 | 0 | 9 | 0 | same |
| 23 state_query | 396 | 396 | 0 | 0 | 0 | same |
| 24 uniform_api | 1267 | 1267 | 0 | 0 | 0 | same |
| 25 clipping | 602 | 602 | 0 | 0 | 0 | same |
| 26 vertex_arrays | 571 | 571 | 0 | 0 | 0 | same |
| 27 draw, attribute_location, rasterization, buffer | 259 | 254 | 0 | 0 | 5 | same |
| 28 default_vertex_attrib ... multisampled_render_to_texture | 260 | 235 | 0 | 25 | 0 | same |

Every row's SNVS record word is identical to r2's except e21, whose
fail-count / CRC words changed (`002e0299,00000000,8a0502c7` against
`000002c7,00000000,ffff02c7`), and the 8 new `e21-p*` name pages.

## Verdict lines (run 1)

Every row's line as `tools/board.sh` printed it (no address in them):

```
board.sh imx8mp e00 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_CASES resets=0 record=594b0909,00015555,00000000,ffff00ff back_after=50s uptime=31s
board.sh imx8mp e01 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_CASES resets=0 record=594b0913,55555555,00000015,ffff00ff back_after=64s uptime=31s
board.sh imx8mp e02 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0890,000005e8,00000000,ffff05e8 back_after=96s uptime=26s
board.sh imx8mp e03 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0884,000003e2,00000000,ffff03e2 back_after=117s uptime=26s
board.sh imx8mp e04 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08e8,00000494,00000000,ffff0494 back_after=104s uptime=29s
board.sh imx8mp e05 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0858,0000044c,00000000,ffff044c back_after=109s uptime=21s
board.sh imx8mp e06 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a2,00000271,00000000,ffff0271 back_after=102s uptime=30s
board.sh imx8mp e07 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0898,0000076c,00000000,ffff076c back_after=134s uptime=21s
board.sh imx8mp e08 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b080c,00000526,00000000,ffff0526 back_after=109s uptime=27s
board.sh imx8mp e09 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b087e,0000065f,00000000,ffff065f back_after=141s uptime=29s
board.sh imx8mp e10 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0810,00000028,00000000,ffff0028 back_after=82s uptime=25s
board.sh imx8mp e11 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08d0,00000088,00000000,ffff0088 back_after=104s uptime=27s
board.sh imx8mp e12 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a0,00000070,00000000,ffff0070 back_after=91s uptime=26s
board.sh imx8mp e13 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0808,00000024,00000000,ffff0024 back_after=95s uptime=22s
board.sh imx8mp e14 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a0,00000062,000e0000,ffff0070 back_after=90s uptime=28s
board.sh imx8mp e14-p0 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a0e,72fd0234,06028a0f,3247f0f0 back_after=103s uptime=28s
board.sh imx8mp e14-p1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a0e,9f6fbcb7,e44d514b,f0dc92fe back_after=90s uptime=28s
board.sh imx8mp e14-p2 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a0e,545de690,ffffffff,ffffffff back_after=102s uptime=26s
board.sh imx8mp e15 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0860,00000050,00000000,ffff0050 back_after=103s uptime=27s
board.sh imx8mp e16 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0840,000000c0,00000000,ffff00c0 back_after=91s uptime=24s
board.sh imx8mp e17 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0874,0000005a,00000000,ffff005a back_after=104s uptime=27s
board.sh imx8mp e18 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0834,0000003a,00000000,ffff003a back_after=94s uptime=20s
board.sh imx8mp e19 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0808,00000424,00000000,ffff0424 back_after=96s uptime=28s
board.sh imx8mp e20 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b087e,0000035f,00000000,ffff035f back_after=156s uptime=25s
board.sh imx8mp e21 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b084e,002e0299,00000000,8a0502c7 back_after=102s uptime=29s
board.sh imx8mp e21-p0 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a2e,2e058a05,8aa7bda8,4f8a9af6 back_after=107s uptime=22s
board.sh imx8mp e21-p1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a2e,4f626e98,a01c5d86,cf84bfe4 back_after=104s uptime=30s
board.sh imx8mp e21-p2 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a2e,863dffeb,3594d3e3,24bc3e63 back_after=104s uptime=19s
board.sh imx8mp e21-p3 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a2e,5e37d18a,c99d96a3,331e8109 back_after=102s uptime=28s
board.sh imx8mp e21-p4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a2e,54909ccd,05ccc691,6b47cb26 back_after=108s uptime=22s
board.sh imx8mp e21-p5 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a2e,bb6a3796,bb07a249,61d6a1ed back_after=102s uptime=29s
board.sh imx8mp e21-p6 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a2e,0355b967,c30d36a5,e80c6ab7 back_after=107s uptime=22s
board.sh imx8mp e21-p7 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a2e,890413d6,daa6f70a,ffffffff back_after=103s uptime=30s
board.sh imx8mp e22 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08ac,000000ed,00000009,ffff00f6 back_after=91s uptime=21s
board.sh imx8mp e23 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08d8,0000018c,00000000,ffff018c back_after=82s uptime=25s
board.sh imx8mp e24 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a6,000004f3,00000000,ffff04f3 back_after=115s uptime=27s
board.sh imx8mp e25 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0874,0000025a,00000000,ffff025a back_after=95s uptime=30s
board.sh imx8mp e26 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0836,0000023b,00000000,ffff023b back_after=145s uptime=21s
board.sh imx8mp e27 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08c6,000000fe,00050000,ffff0103 back_after=104s uptime=31s
board.sh imx8mp e27-p0 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a05,4b3a5417,db733a1d,ffff9f30 back_after=109s uptime=26s
board.sh imx8mp e28 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08c8,000000eb,00000019,ffff0104 back_after=265s uptime=31s
TALLY (29 entries): begun 17371 passed 17272 failed 46 notsup 34 other 19 noresult 0
```

## How it was run

The board run needs `BOARD_HOST`, `BOARD_BOOT` and `TREE_IMAGE` in the environment.
The sandbox refuses a command with that prefix, so a wrapper script on the worker
(outside the repository) exports the three variables (`BOARD_HOST` from the leased
resource's `RESOURCE_*_SSH`) and runs, from the workspace at `cd738b1a`:

```sh
tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt records/imx8mp/deqp-cd738b1a
tools/board.sh imx8mp tally records/imx8mp/deqp-cd738b1a
diff tools/cts/imx8mp-full-tree-29460f96.results.tsv records/imx8mp/deqp-cd738b1a/results.tsv
```

The records (`results.tsv`, `rows.md`, a directory per row) stay under `records/`
on the worker and are not committed. One tree took about 75 minutes.
