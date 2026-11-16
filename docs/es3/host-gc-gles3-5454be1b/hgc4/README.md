# HOST-ONLY GC7000UL dEQP-GLES3, part 4: functional.texture

**HOST-ONLY.** Every number here comes from a run on a host with the Linux
OSAL, which keeps the command stream instead of executing it: every
`glReadPixels` reads black, so every render verdict (Pass, or Fail by image
comparison or verification) is meaningless and must not be quoted as a board
result. What is real is each case's compile and link outcome, its GL errors
and its NotSupported reason.

- Commit (library and harness): `5454be1b6bf561d9ed3ee686c5e039bc323147c4`
- GPU: vivante (the GC7000UL's online back end), module gles3; the library
  reports OpenGL ES 3.0 (`YAKOGL_REPORT_ES2=OFF`, which `MODULE=gles3` sets)
- VK-GL-CTS: `93bca01861b0e3ef3c387027a9791e6d065f900c` with
  `tools/cts/cts-yakogl-target.patch` applied; case list: the union of its
  `android/cts/main/gles3-main-*.txt`
- Run on 2026-11-16, default chunking (100 cases per process)
- Command, from a worktree of the commit:

```
GPU=vivante MODULE=gles3 REPORT_ES2=0 CTS=<deps>/VK-GL-CTS OUT=<deps>/hgc-5454be1b/hgc4 \
  tools/cts/host-deqp-run.sh dEQP-GLES3.functional.texture
```

(`REPORT_ES2` is not read by the script; the module sets it.)

## Result (HOST-ONLY)

| | cases |
|---|---|
| asked (mustpass names containing `dEQP-GLES3.functional.texture`) | 5105 |
| begun | 5105 |
| not executed | 0 |
| aborted chunks (`aborted.txt`) | 0 (empty) |

Process exits: 11 with exit 0, 41 with exit 1 (a chunk with a failing case
exits 1); no chunk needed more than one process.

Status of the 5105 begun cases (HOST-ONLY): Pass 19, Fail 3748,
NotSupported 1338, Crash 0, other 0 (19 + 3748 + 1338 = 5105).

Fail by message (HOST-ONLY):

| cases | message | kind |
|---|---|---|
| 2096 | Image comparison failed | render, meaningless on the host |
| 1084 | Image verification failed | render, meaningless on the host |
| 432 | Failed to compile shader program (glsTextureTestUtil.cpp:411) | compile |
| 68 | `glGetError() == GL_NO_ERROR` (es3fTextureSpecificationTests.cpp:220) | GL error |
| 33 | Set 2d / 2d array / 3d / cube image data: `GL_INVALID_OPERATION` (es3fTextureUnitTests.cpp) | GL error |
| 32 | Texture upload failed: `GL_INVALID_OPERATION` (gluTexture.cpp:188, 418, 633, 755; 8 each) | GL error |
| 3 | `checkFramebufferStatus == GL_FRAMEBUFFER_COMPLETE` (es3fTextureSpecificationTests.cpp:3626) | GL error |

The 432 compile failures are one refusal, the whole of
`functional.texture.shadow`: the code generator refused the fragment shader
with `Select(Unsupported("an ES 3.00 texture look-up (a shadow sampler, an
integer result, a gradient, an offset or a cube fetch) is not selected
yet"))`.

NotSupported by reason: 1274 `ASTC not supported`, 64
`GL_EXT_texture_sRGB_R8` / `GL_EXT_texture_sRGB_RG8 is not supported`.

For comparison, `docs/es3/host-gles3-tip.tsv` has vivante
`functional.texture` as 19 / 3428 / 1658 (pass / fail / notsupported) at its
earlier tip; at 5454be1b 320 cases moved from NotSupported to Fail, HOST-ONLY
both.

## Files

- `texture.part`: `tools/cts/host-gles3-tally.py --part` of the run
  directory, the layout of the other parts (hgc1..hgc6) for the merge: one tip
  row (`functional.texture`) and the counting not-pass rows (refusal,
  gl-error, compile; render Fails are not listed). HOST-ONLY.
- `group-counts.tsv`: per subgroup of `functional.texture` and a total row;
  columns of `docs/es3/host-gles3-tip.tsv` (gpu group pass fail notsupported
  crash other). HOST-ONLY.
- `notpass.tsv`: `case<TAB>status` for all 5086 cases that are not Pass
  (3748 Fail + 1338 NotSupported). HOST-ONLY.
