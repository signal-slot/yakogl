# GL_OES_texture_npot and dEQP-GLES2: what the CTS source checks

Item `es2-npot-research` of plan #4, 2026-10-08. Host-only reading; no board run, no code run, no source or doc other than this file changed.

Sources read:

- VK-GL-CTS checkout `deps/yakogl/VK-GL-CTS` (commit not recorded; `git log` was not permitted in this session), `modules/gles2/functional/`: `es2fTextureCompletenessTests.cpp` (CC below), `es2fNegativeTextureApiTests.cpp` (NT), `es2fTextureWrapTests.cpp`, `es2fTextureFormatTests.cpp`, `es2fTextureFilteringTests.cpp`, `es2fTextureSizeTests.cpp`, `es2fFboRenderTest.cpp`, `es2fApiCase.cpp`. A grep for `npot`, `PowerOfTwo` and the extension check over all of `modules/gles2` found no other user (`tes2CapabilityTests.cpp` only prints the extension name; `accuracy/` and `performance/` are not in the mustpass).
- `tools/cts/gles2-mustpass.txt` (117 names contain `npot`; plus 3 + 4 NPOT-sized cases without it, section 3).
- yakogl at the integration tip `7bc37ed`: `src/gl/texture.rs`, `src/gl/draw.rs`, `src/gl/state.rs`, and the unit tests named in `docs/es2-cts-failures.md` section H.

## 1. Result in one paragraph

**The CTS never throws NotSupported for GL_OES_texture_npot.** The section H prediction ("Pass -> NotSupported", 18 cases) is wrong. With the extension listed, the 10 wrap cases and `2d.npot_size` are written to **render blue** (the extension "makes the completeness test pass"), `generatemipmap_npot_wdt_hgt` **returns early and is a plain Pass**, and the other 6 size cases still **expect black and no GL error**. So the extension turns the cases into real tests of our NPOT sampling instead of skipping them: 0 NotSupported, 18 cases that must Pass, and the cost of a mistake is a **Fail**, not a skip.

yakogl at the tip is, in the host model, written to pass all 18 (section 4). The residual risk is in what a board does, not in the GL core: PowerVR's TPU mip addressing for an NPOT chain (unmeasured), the GC7000 REPEAT on an NPOT size (`NON_POWER_OF_TWO` feature bit unconfirmed), and the Pi 4 is expected to be fine (Mesa's V3D claims the extension and runs these cases).

## 2. The check the CTS makes

| Where | What |
|---|---|
| CC:67-76 `isExtensionSupported(ctxInfo, ext)` | exact string match over `getExtensions()` |
| CC:291-309 | `Incomplete2DSizeCase`: relaxing extensions are `GL_OES_texture_npot` or `GL_NV_texture_npot_2D_mipmap`; first one listed wins |
| CC:332-334 | per `glTexImage2D`: expects `GL_INVALID_VALUE` only if **no** relaxing extension, level > 0 and the level is not POT; else `GL_NO_ERROR` (a failure is `TCU_FAIL`, via `expectError` CC:78-88) |
| CC:338-343 | only if the **base** size is NPOT and an extension is listed: expected colour becomes blue `(0,0,255,255)`. Otherwise it stays black `(0,0,0,255)` (CC:150) |
| CC:517-523 / CC:920-926 | wrap cases (2D / cube): extension listed -> expected colour blue |
| CC:671, 701-703 | `IncompleteCubeSizeCase`: only `GL_OES_texture_npot` (not the NV one); error expectation as CC:332-334; **never changes the expected colour: black** |
| NT:1341-1349 | `generatemipmap_npot_wdt_hgt`: extension listed -> log line and `return` |

Nothing throws `NotSupportedError` on the extension. The `NotSupportedError`s in the files read are unrelated (small viewport, `GL_EXT_color_buffer_half_float`, `GL_EXT_texture_format_BGRA8888`). `es2fApiCase.cpp:54` sets the result to Pass before the case body runs, so the early `return` at NT:1348 is **Pass** (the log says "skipping test", but the result is not `NotSupported`).

Test body, all `completeness` cases: draw one textured quad over a 64x64 viewport (`Tex2DCompletenessCase::iterate`, CC:154-186; cube: six faces, CC:216-254), every pixel must equal `m_compareColor` exactly (`compareToConstantColor`, CC:90-122). The texture is solid blue `(0,0,1,1)`.

## 3. Every NPOT-dependent mustpass case

Verdicts. Pass = expected to Pass; Risk = Pass on the host, a board could Fail. No case is NotSupported in any column. "PVR" = PowerVR (AM62P, base-level-only before 2026-10-08, now a pot-extent chain for 2D, base level only for cubes), "GC" = GC7000 (i.MX8MP), "V3D" = Pi 4.

### 3.1 Cases that change with the extension (the 18 of section H)

yakogl today for all of these: `NPOT_EXTENSION` is true and `NPOT_RESTRICTED` is false in every `REPORT_ES2` build (`src/gl/texture.rs:855`, `state.rs`), so the rules of `draw.rs:2351` (`incomplete_reason`, the only NPOT wrap rule) and `texture.rs:2062`, `texture.rs:3305` are off.

| Case (`dEQP-GLES2.functional.` ...) | CTS behaviour | yakogl today (host model) | PVR | GC | V3D |
|---|---|---|---|---|---|
| `texture.completeness.2d.npot_t_repeat` | CC:1062; 127x127 level 0 only, S clamp / T repeat, NEAREST (CC:504-512); ext -> blue (CC:517-523); no error check | complete (`incomplete_reason` has no NPOT arm); one level sampled; test `a_npot_texture_is_complete_only_where_the_version_allows_it` (`draw.rs:12038`) pins `incomplete == 0` | Pass | Risk (REPEAT on NPOT needs `chipMinorFeatures1_NON_POWER_OF_TWO`, unconfirmed; solid colour makes the case insensitive unless the last column is fetched out of range) | Pass |
| `...2d.npot_s_repeat` | CC:1065; same, S repeat / T clamp | same | Pass | Risk (as above) | Pass |
| `...2d.npot_all_repeat` | CC:1068; REPEAT / REPEAT | same | Pass | Risk | Pass |
| `...2d.npot_mirrored_repeat` | CC:1070; MIRRORED_REPEAT both | same | Pass | Risk | Pass |
| `...2d.repeat_width_npot` | CC:1073; 127x128 REPEAT | same | Pass | Risk | Pass |
| `...2d.repeat_height_npot` | CC:1075; 128x127 REPEAT | same | Pass | Risk | Pass |
| `...cube.npot_t_repeat` | CC:1125; 127x127 on six faces; **the case binds the new texture to `GL_TEXTURE_2D` (CC:904) and sets parameters and images on the `GL_TEXTURE_CUBE_MAP` binding, i.e. the default cube texture, name 0**; ext -> blue (CC:920-926) | needs the default cube object to take images and be sampled: `a_sampler_reads_the_default_texture_object_at_binding_0` (`draw.rs:10950`, fix of 2026-09-24, cube.extra_level has the same shape); NPOT wrap rule off, so complete | Pass (cube keeps base level, one level is all a NEAREST case needs) | Risk (as 2D; cube sampled by hardware) | Pass |
| `...cube.npot_s_repeat` | CC:1128 | same | Pass | Risk | Pass |
| `...cube.npot_all_repeat` | CC:1131 | same | Pass | Risk | Pass |
| `...cube.npot_mirrored_repeat` | CC:1133 | same | Pass | Risk | Pass |
| `...texture.completeness.2d.npot_size` | CC:1035; 255x255 base, every level the right size (`max(1, 255>>l)`: 255 .. 1, eight levels, CC:320-326), NEAREST_MIPMAP_NEAREST, REPEAT; ext -> **no `glTexImage2D` error at levels 1-7** (CC:332-334) and **blue** (CC:338-343): a complete NPOT mip chain must be sampled | level 1 (127) is accepted, `grow_npot_chain` lays the chain out (`texture.rs:2117-2135`), levels 2-7 match `level_size`, complete; pinned at 15x15 by `a_npot_level_above_0_follows_the_version` (`draw.rs:12225`) and `a_npot_level_above_0_grows_the_chain_where_the_backend_holds_one` (`texture.rs:8098`); never run at 255 | Risk: 2D chain is the 256-extent pot layout read by the TPU's own arithmetic (unmeasured for the 63x63 level 2 the 64x64 viewport selects) | Pass (named per-level addresses) | Pass (Mesa slices) |
| `...2d.npot_size_level_0` | CC:1037; 256x256 base, level 0 is **255x255**, levels 1.. are 128, 64 ..; base is POT -> stays black (CC:338 false); no error at any level (levels 1-8 are POT) | level 1 is 128, GL's chain of 255 has 127: `width != lw` -> `level_bad`; incomplete -> black. The 1x1 at level 8 is `past_the_chain` and ignored (`texture.rs:2160-2171`) | Pass | Pass | Pass |
| `...2d.npot_size_level_1` | CC:1039; 256 base, level 1 is **127x127**; ext -> **no error** (without: `GL_INVALID_VALUE`, CC:332-334); black | no error (`NPOT_RESTRICTED` false); 127 != 128 -> recorded bad, incomplete -> black | Pass | Pass | Pass |
| `...cube.npot_size_level_0` | CC:1084; 64x64 faces, level 0 is 63x63 on all six; levels 1.. are 32, 16 ..; `fullNpotSupport` -> no error; **black** | 32 != 31 -> `level_bad`; incomplete -> black; no all-faces-63 test, but the sibling `a_cube_face_of_another_size_at_level_0_leaves_it_incomplete` (`texture.rs:10962`, 7 among 8) covers the per-face record | Pass | Pass | Pass |
| `...cube.npot_size_level_1` | CC:1086; level 1 is 31x31 on all faces; no error; black | 31 vs 32 never fits; bad; black | Pass | Pass | Pass |
| `...cube.npot_size_level_0_pos_x` | CC:1088; only +X level 0 is 63x63; black | the other faces are 64: `faces_set` loses +X's record, the per-face level record reads incomplete; `a_cube_face_of_another_size_at_level_0_leaves_it_incomplete` | Pass | Pass | Pass |
| `...cube.npot_size_level_1_neg_x` | CC:1091; only -X level 1 is 31x31; black | `level_faces` per face; `a_cube_face_missing_or_misfit_at_level_1_leaves_it_incomplete` (`texture.rs:11008`) | Pass | Pass | Pass |
| `...negative_api.texture.generatemipmap_npot_wdt_hgt` | NT:1341-1349: extension listed -> log + `return`, no GL call; result already Pass (`es2fApiCase.cpp:54`) | not exercised; `glGenerateMipmap` of NPOT is `GL_NO_ERROR` (`texture.rs:3305`, `generate_mipmap_of_a_npot_texture_follows_the_reported_version`, `texture.rs:11077`) | Pass | Pass | Pass |

Result: 17 cases render and compare, 1 case does not run its body. The 7 "expects black" cases are Pass only because yakogl checks the chain's level sizes (`incomplete_reason`, `draw.rs:2316-2349`) and does not refuse the upload; a build that returned `GL_INVALID_VALUE` for an NPOT level (the ES 2.0 rule without the extension) would **fail** them now (CC:334 and CC:703 expect `GL_NO_ERROR`), which is a second reason not to take the extension out in only one place.

### 3.2 NPOT-sized, extension-independent (unchanged)

One level or no mipmap, CLAMP_TO_EDGE: complete under ES 2.0 with or without the extension, nothing in the test reads the extension list. Listed so the item does not touch them.

| Cases | Why unchanged (CTS) |
|---|---|
| `texture.format.{a8,l8,la88,rgb565,rgb888,rgba4444,rgba5551,rgba8888}_{2d,cube}_npot`, `etc1_{2d,cube}_npot` (18) | clamp, NEAREST, one level: TF:182-184, 373-375, 498-500, 622-624; sizes 63x112, 57x57, ETC1 113x89 / 61x61 (TF:693-739) |
| `texture.filtering.{2d,cube}.*_clamp_*_npot` (32) | name says clamp; sizes in the file are POT-only for mipmap (TFL:184, 462: `mipmaps` only when POT) |
| `texture.wrap.clamp_clamp_*_npot_*` (8) | TW:313-355: the generator skips every non-clamp NPOT combination ("Not supported as described in Spec section 3.8.2"), so a REPEAT NPOT wrap case does not exist in the mustpass; ETC1 113x89 clamp: TW:380-391 |
| `texture.size.2d.65x63_{l8,rgb888,rgba8888}`, `texture.size.cube.15x15_{l8,rgb888,rgba4444,rgba8888}` (7) | `isPOT` false -> no mipmap (TS:390, 413); clamp, LINEAR/NEAREST (TS:140-156, 309-311) |
| `fbo.render.{color,depth,stencil}.*npot*` (41) | NPOT FBO textures set CLAMP_TO_EDGE and LINEAR (FR:497-501); the renderbuffers have no wrap |

## 4. yakogl per back end (what decides the table above)

- **Extension and rules.** `state::NPOT_EXTENSION` true, `texture::NPOT_RESTRICTED = REPORT_ES2 && !NPOT_EXTENSION` false in every build (`texture.rs:855`). Consequences: no NPOT wrap/filter rule in `incomplete_reason` (`draw.rs:2351`), no `GL_INVALID_VALUE` for an NPOT level (`texture.rs:2062`), no `GL_INVALID_OPERATION` in `glGenerateMipmap` (`texture.rs:3305`).
- **Who holds an NPOT chain** (`npot_mip_chains`, `texture.rs:871`): GC7000 and V3D (native layered textures: whole chain, level i = `max(w>>i, 1)`), PowerVR for a 2D texture (since `cc78859`, 2026-10-08: laid out as the chain of its power-of-two extent, `chain_layout_npot`, `pot_chain`). A PowerVR **cube** keeps its base level and records GL's chain as present (`a_powervr_npot_cube_keeps_its_base_level`, `texture.rs:7928`); it samples level 0 under a mip filter. No case of 3.1 samples a mipmapped NPOT cube (the cube size cases expect black, the cube wrap cases use NEAREST), so that is invisible to dEQP.
- **Section H is out of date on the PowerVR** ("base level only"): `cc78859` made it a pot-extent chain for 2D after H was written. That does not change a verdict but it does move the PowerVR risk from "sampled from level 0" to "the TPU's own level arithmetic for NPOT sizes", which only `2d.npot_size` exercises.
- **Default texture object 0.** The cube wrap cases (CC:904-916) need images specified on an unbound-by-name cube target to be sampled; supported (`draw.rs:10950` test). Not checked on a board for NPOT 127.

## 5. Differences from the section H prediction

| Section H says | CTS source says |
|---|---|
| 10 wrap cases: Pass -> NotSupported ("the recollection that ... skips its NPOT cases with NotSupported") | Not skipped. With the extension they compare against **blue** (CC:517-523, CC:920-926). Verdict **Pass**, no NotSupported. If a case Fails, it is a real sampling or completeness fault. |
| `2d.npot_size`: Pass -> NotSupported, or stays Pass | Runs; must be **blue** (CC:338-343) and every `glTexImage2D` must be `GL_NO_ERROR` (CC:332-334). It is the only case that needs a complete NPOT mip chain sampled correctly. |
| `2d.npot_size_level_0`, `npot_size_level_1`: Pass -> NotSupported or Pass | Run; expect **black**, no error. Pass (they depend on yakogl checking level sizes). |
| 4 cube size cases: Pass -> NotSupported or Pass | Run; expect black, no error (`fullNpotSupport`, CC:671, 701-703). Pass. |
| `generatemipmap_npot_wdt_hgt`: Pass -> NotSupported | Early `return` (NT:1344-1349), result is Pass (`es2fApiCase.cpp:54`). Never `NotSupported`. |
| Predicted totals: Pass -18, NotSupported +18 per board (AM62P 19 -> 37, i.MX8MP 34 -> 52, Pi 4 62 -> 80) | **NotSupported unchanged** (19 / 34 / 62), **Pass unchanged**, failures 0 expected. Only a Fail moves a number. |
| "A case that failed whenever the extension is listed would be failing on every such driver" | True and it is the whole point: Mesa V3D and NXP pass these as blue/black cases; the cases are written for drivers that claim the extension (the `GL_NV_texture_npot_2D_mipmap` line, CC:291-309, is the other relaxing extension for the 2D size case only). |
| "No ES 2.0 mustpass case samples a mipmapped NPOT texture" | `2d.npot_size` does (255x255, NEAREST_MIPMAP_NEAREST, REPEAT). |
| PowerVR "base level only" | Out of date: 2D is a pot-extent chain since 2026-10-08. |
| Other `*_npot` cases unchanged | Confirmed (3.2), with 7 more NPOT-sized cases without `npot` in the name (`texture.size.*`). |
| Board check list (completeness, generatemipmap, wrap/format/filtering/size/mipmap) | Right set; the pass criterion is "all Pass", not "18 NotSupported". Fail on a wrap case does not mean "take the extension out": it means the NPOT REPEAT sample or the chain is wrong on that GPU. |

## 6. What the `es2-npot-cases` fix item must do

The GL core already follows the CTS; there is no case to change from NotSupported expectations. Work, in order:

1. **Docs, not code**: replace section H's table and totals with section 3.1 / 5 of this file (the item that edits `docs/es2-cts-failures.md`; this research was told not to).
2. **Host proof of the exact CTS sequences** (`src/gl/texture.rs` tests, `src/gl/draw.rs` tests): none of them runs at the CTS sizes. Add tests that replay, via the test GL (`TestGl` / `Gl::new`), `Incomplete2DSizeCase::createTexture` and `IncompleteCubeSizeCase::createTexture` literally, with the sizes of CC:1035-1040, 1084-1093: assert **no GL error in any call** and `incomplete` == 0 (`npot_size`, 255) or 1 (the other six), on the mock of each back end (`Gl::backend_like_gc7000()` is there; add the V3D shape and the PowerVR default). Also replay `Incomplete2DWrapModeCase` and `IncompleteCubeWrapModeCase` (texture bound to 2D, parameters and faces on the cube binding, CC:895-916) and assert `incomplete == 0` for the six/four sizes and wraps.
3. **Fix stale comments and test names** that still describe the ES 2.0 error as what dEQP expects: `texture.rs:2053`, `texture.rs:6582` and `draw.rs:12220` ("dEQP texture.completeness npot_size expects it"), `cube_chain_is_complete` doc at `texture.rs:10784-10786` ("which dEQP expects too"). The CTS expects `GL_NO_ERROR` when the extension is listed.
4. **`a_npot_level_above_0_follows_the_version` / `tex_image_level_on_npot_texture_follows_the_version`**: keep, but the `restricted` branch is dead; either delete it or keep it as the revert check of the version rules.
5. **Behaviour to verify for `2d.npot_size` (PVR)**: in `src/gl/texture.rs` (`chain_layout_npot`, `pot_chain_layout`, `grow_npot_chain`) and the PowerVR backend descriptor (`src/pvr/`): the TPU reads level 2 of a 255x255 texture (63x63 at the 64-texel pitch). If the descriptor passes the pot width (64) rather than 63 for the level, the sample can reach the unwritten column 63 (black). A host test cannot see this; the item should record it as the board check below.
6. **Board check (one run per board)**, the same list as section H, with the criterion changed: `texture.completeness.*` (17 cases), `negative_api.texture.generatemipmap_npot_wdt_hgt`, `texture.{wrap,format,filtering,size}.*`: all **Pass**, none NotSupported. Watch first `completeness.2d.npot_size` (AM62P) and `completeness.2d.npot_*_repeat` (i.MX8MP). If a repeat case Fails on the GC7000: read `minor_features[1]` (not recorded), and test a REPEAT-wrapped NPOT draw; the choice is then between leaving the extension claimed and a Fail on that board, or taking it out for that GPU (`NPOT_EXTENSION` is one constant for all GPUs today, so that is a code change, and it re-opens glmark2 terrain's black texture): record it for the owner, do not decide it in the item.
7. Nothing in `src/gl/state.rs` changes: the extension stays listed. Without it the same cases expect `GL_INVALID_VALUE` at level > 0 and black wrap textures, which the code gives only if `NPOT_RESTRICTED` is true in `texture.rs` and `draw.rs` together.

## 7. Not checked

- Board behaviour of any case (no run). All verdicts are source reading plus the host tests named.
- The CTS commit: the checkout's `git log` was not permitted; the mustpass is `tools/cts/gles2-mustpass.txt` and the case names of 3.1 were matched against `TextureCompletenessTests::init` (CC:1027-1139).
- Whether `dEQP` reports the early return of NT:1348 as Pass in the qpa for this runner; `es2fApiCase.cpp:54` sets Pass first.
- `texture.unit`, `vertex_texture`, `mipmap` and `texture.wrap.*` POT cases: no extension check, POT sizes only.
