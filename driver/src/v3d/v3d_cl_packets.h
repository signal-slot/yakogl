/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The V3D 4.2 packets and records YakoGL writes, by their names in Mesa's
 * v3d_packet.xml (struct V3D42_<name>), as an X-macro: v3d_cl.h makes a
 * checked packer of each, and tools/v3d/cl_golden.c checks each against
 * Mesa's own build. No includes: a checker includes it next to Mesa's
 * header as well as next to ours.
 *
 * V3D71_CL_PACKETS is the same table for V3D 7.1 (the Pi 5, struct
 * V3D71_<name>), made from the 4.2 one by the `min_ver="71"` entries of
 * the XML (docs/notes/pre-study-rpi5.md, 3.2): CLEAR_TILE_BUFFERS is
 * CLEAR_RENDER_TARGETS, TILE_RENDERING_MODE_CFG_COLOR and the three CLEAR_COLORS
 * parts are the three RENDER_TARGET parts, and the packets 7.1 adds are
 * listed. No driver code writes 7.1 yet: only tools/v3d/cl_golden.c builds
 * it, with -DV3D_CL_VER=71. V3D_CL_PACKET_LIST is the table of that build.
 */
#ifndef V3D_CL_PACKETS_H
#define V3D_CL_PACKETS_H

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
	X(CLEAR_TILE_BUFFERS)                                                                      \
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
	X(TILE_BINNING_MODE_CFG)                                                                   \
	X(TILE_RENDERING_MODE_CFG_COMMON)                                                          \
	X(TILE_RENDERING_MODE_CFG_COLOR)                                                           \
	X(TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES)                                                 \
	X(TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1)                                              \
	X(TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART2)                                              \
	X(TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART3)                                              \
	X(TILE_COORDINATES)                                                                        \
	X(MULTICORE_RENDERING_SUPERTILE_CFG)                                                       \
	X(MULTICORE_RENDERING_TILE_LIST_SET_BASE)                                                  \
	X(TILE_COORDINATES_IMPLICIT)                                                               \
	X(TILE_LIST_INITIAL_BLOCK_SIZE)                                                            \
	X(GL_SHADER_STATE_RECORD)                                                                  \
	X(GL_SHADER_STATE_ATTRIBUTE_RECORD)                                                        \
	X(TEXTURE_SHADER_STATE)                                                                    \
	X(SAMPLER_STATE)

#define V3D71_CL_PACKETS(X)                                                                        \
	X(HALT)                                                                                    \
	X(FLUSH)                                                                                   \
	X(START_TILE_BINNING)                                                                      \
	X(END_OF_RENDERING)                                                                        \
	X(RETURN_FROM_SUB_LIST)                                                                    \
	X(FLUSH_VCD_CACHE)                                                                         \
	X(START_ADDRESS_OF_GENERIC_TILE_LIST)                                                      \
	X(BRANCH_TO_IMPLICIT_TILE_LIST)                                                            \
	X(SUPERTILE_COORDINATES)                                                                   \
	X(CLEAR_RENDER_TARGETS)                                                                    \
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
	X(DEPTH_BOUNDS_TEST_LIMITS)                                                                \
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
	X(CLIPPER_Z_SCALE_AND_OFFSET_NO_GUARDBAND)                                                 \
	X(TILE_BINNING_MODE_CFG)                                                                   \
	X(TILE_RENDERING_MODE_CFG_COMMON)                                                          \
	X(TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES)                                                 \
	X(TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1)                                             \
	X(TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART2)                                             \
	X(TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART3)                                             \
	X(TILE_COORDINATES)                                                                        \
	X(MULTICORE_RENDERING_SUPERTILE_CFG)                                                       \
	X(MULTICORE_RENDERING_TILE_LIST_SET_BASE)                                                  \
	X(TILE_COORDINATES_IMPLICIT)                                                               \
	X(TILE_LIST_INITIAL_BLOCK_SIZE)                                                            \
	X(GL_SHADER_STATE_RECORD)                                                                  \
	X(GL_SHADER_STATE_RECORD_DRAW_INDEX)                                                       \
	X(GL_SHADER_STATE_ATTRIBUTE_RECORD)                                                        \
	X(TEXTURE_SHADER_STATE)                                                                    \
	X(SAMPLER_STATE)

#if defined(V3D_CL_VER) && V3D_CL_VER == 71
#define V3D_CL_PACKET_LIST V3D71_CL_PACKETS
#else
#define V3D_CL_PACKET_LIST V3D_CL_PACKETS
#endif

#endif
