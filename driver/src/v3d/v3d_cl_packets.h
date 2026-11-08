/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The V3D packets and records YakoGL writes, by their names in Mesa's
 * v3d_packet.xml (struct V3D<ver>_<name>), as an X-macro: v3d_cl.h makes a
 * checked packer of each, and tools/v3d/cl_golden.c checks each against
 * Mesa's own build. No includes: a checker includes it next to Mesa's
 * header as well as next to ours.
 *
 * The generation is V3D_CL_VER: 42 (the Pi 4's V3D 4.2, the default, what
 * the driver builds) or 71 (the Pi 5's V3D 7.1). One table serves both.
 * X(name) is a packet of both generations; V3D_CL_X42(X, name) and
 * V3D_CL_X71(X, name) are one generation's and expand to nothing in the
 * other's build, so the 4.2 table is the table it was. Where 7.1 renames a
 * 4.2 packet the two stand side by side (docs/notes/pre-study-rpi5.md 3.2,
 * docs/notes/v3d71-clear-frame.md): CLEAR_TILE_BUFFERS is
 * CLEAR_RENDER_TARGETS (no flags), and TILE_RENDERING_MODE_CFG_COLOR with
 * the three CLEAR_COLORS parts are the three RENDER_TARGET parts, one set
 * per render target. NUMBER_OF_LAYERS exists at both generations and is
 * written at 7.1 only: Mesa's 7.1 clear frame starts its binning list with
 * it and the Pi 5 probes follow that frame; the 4.2 clear frame does not
 * emit it, and the 4.2 table (and the case count tools/check.sh pins for
 * it) stays as it is.
 */
#ifndef V3D_CL_PACKETS_H
#define V3D_CL_PACKETS_H

#ifndef V3D_CL_VER
#define V3D_CL_VER 42
#endif
#if V3D_CL_VER == 42
#define V3D_CL_X42(X, name) X(name)
#define V3D_CL_X71(X, name)
#elif V3D_CL_VER == 71
#define V3D_CL_X42(X, name)
#define V3D_CL_X71(X, name) X(name)
#else
#error "V3D_CL_VER is 42 or 71"
#endif

#define V3D_CL_PACKETS(X)                                                                          \
	X(HALT)                                                                                    \
	X(FLUSH)                                                                                   \
	X(START_TILE_BINNING)                                                                      \
	X(END_OF_RENDERING)                                                                        \
	X(RETURN_FROM_SUB_LIST)                                                                    \
	X(FLUSH_VCD_CACHE)                                                                         \
	X(START_ADDRESS_OF_GENERIC_TILE_LIST)                                                      \
	X(BRANCH_TO_IMPLICIT_TILE_LIST)                                                            \
	X(SUPERTILE_COORDINATES)                                                                   \
	V3D_CL_X42(X, CLEAR_TILE_BUFFERS)                                                          \
	V3D_CL_X71(X, CLEAR_RENDER_TARGETS)                                                        \
	X(END_OF_LOADS)                                                                            \
	X(END_OF_TILE_MARKER)                                                                      \
	X(STORE_TILE_BUFFER_GENERAL)                                                               \
	X(LOAD_TILE_BUFFER_GENERAL)                                                                \
	X(INDEXED_PRIM_LIST)                                                                       \
	X(INDEXED_INSTANCED_PRIM_LIST)                                                             \
	X(VERTEX_ARRAY_PRIMS)                                                                      \
	X(VERTEX_ARRAY_INSTANCED_PRIMS)                                                            \
	X(INDEX_BUFFER_SETUP)                                                                      \
	X(SET_INSTANCEID)                                                                          \
	X(PRIM_LIST_FORMAT)                                                                        \
	X(GL_SHADER_STATE)                                                                         \
	X(VCM_CACHE_SIZE)                                                                          \
	X(TRANSFORM_FEEDBACK_SPECS)                                                                \
	X(STENCIL_CFG)                                                                             \
	X(BLEND_ENABLES)                                                                           \
	X(BLEND_CFG)                                                                               \
	X(BLEND_CONSTANT_COLOR)                                                                    \
	X(COLOR_WRITE_MASKS)                                                                       \
	X(ZERO_ALL_CENTROID_FLAGS)                                                                 \
	X(CENTROID_FLAGS)                                                                          \
	X(SAMPLE_STATE)                                                                            \
	X(OCCLUSION_QUERY_COUNTER)                                                                 \
	X(CFG_BITS)                                                                                \
	X(ZERO_ALL_FLAT_SHADE_FLAGS)                                                               \
	X(FLAT_SHADE_FLAGS)                                                                        \
	X(ZERO_ALL_NON_PERSPECTIVE_FLAGS)                                                          \
	X(NON_PERSPECTIVE_FLAGS)                                                                   \
	X(POINT_SIZE)                                                                              \
	X(LINE_WIDTH)                                                                              \
	X(DEPTH_OFFSET)                                                                            \
	X(CLIP_WINDOW)                                                                             \
	X(VIEWPORT_OFFSET)                                                                         \
	X(CLIPPER_Z_MIN_MAX_CLIPPING_PLANES)                                                       \
	X(CLIPPER_XY_SCALING)                                                                      \
	X(CLIPPER_Z_SCALE_AND_OFFSET)                                                              \
	V3D_CL_X71(X, NUMBER_OF_LAYERS)                                                            \
	X(TILE_BINNING_MODE_CFG)                                                                   \
	X(TILE_RENDERING_MODE_CFG_COMMON)                                                          \
	V3D_CL_X42(X, TILE_RENDERING_MODE_CFG_COLOR)                                               \
	X(TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES)                                                 \
	V3D_CL_X42(X, TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1)                                  \
	V3D_CL_X42(X, TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART2)                                  \
	V3D_CL_X42(X, TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART3)                                  \
	V3D_CL_X71(X, TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1)                                 \
	V3D_CL_X71(X, TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART2)                                 \
	V3D_CL_X71(X, TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART3)                                 \
	X(TILE_COORDINATES)                                                                        \
	X(MULTICORE_RENDERING_SUPERTILE_CFG)                                                       \
	X(MULTICORE_RENDERING_TILE_LIST_SET_BASE)                                                  \
	X(TILE_COORDINATES_IMPLICIT)                                                               \
	X(TILE_LIST_INITIAL_BLOCK_SIZE)                                                            \
	X(GL_SHADER_STATE_RECORD)                                                                  \
	X(GL_SHADER_STATE_ATTRIBUTE_RECORD)                                                        \
	X(TEXTURE_SHADER_STATE)                                                                    \
	X(SAMPLER_STATE)

#endif
