/*
 * Copyright © 2022 Imagination Technologies Ltd.
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice (including the next
 * paragraph) shall be included in all copies or substantial portions of the
 * Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */

/* Enums, structures and pack functions for ROGUE.
 *
 * This file has been generated, do not hand edit.
 */

#ifndef IPF_H
#define IPF_H

#include "csbgen/pvr_packet_helpers.h"

#ifndef __OPENCL_VERSION__
#define __constant
#endif


#define ROGUE_IPF_TILE_SIZE_PIXELS               32
#define ROGUE_IPF_CONTROL_STREAM_SIZE_DWORDS     32
#define ROGUE_IPF_ISP_VERTEX_XY_BIAS_VALUE       4096
#define ROGUE_IPF_ISP_VERTEX_XY_BIAS_VALUE_SIPF  2048

enum ROGUE_IPF_COMPRESSION_FORMAT {
    ROGUE_IPF_COMPRESSION_FORMAT_UNIQUE_1 =      0,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_1 =      1,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_2 =      2,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_3 =      3,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_4 =      4,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_5 =      5,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_6 =      6,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_7 =      7,
    ROGUE_IPF_COMPRESSION_FORMAT_RAW_BYTE =      8,
    ROGUE_IPF_COMPRESSION_FORMAT_UNIQUE_2 =      9,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_1 =     10,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_2 =     11,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_3 =     12,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_4 =     13,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_5 =     14,
    ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_6 =     15,
};

static __constant const char *
ROGUE_IPF_COMPRESSION_FORMAT_to_str(const enum ROGUE_IPF_COMPRESSION_FORMAT value)
{
    switch (value) {
    case ROGUE_IPF_COMPRESSION_FORMAT_UNIQUE_1: return "UNIQUE_1";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_1: return "ORIGIN_1_DELTA_1";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_2: return "ORIGIN_1_DELTA_2";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_3: return "ORIGIN_1_DELTA_3";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_4: return "ORIGIN_1_DELTA_4";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_5: return "ORIGIN_1_DELTA_5";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_6: return "ORIGIN_1_DELTA_6";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_1_DELTA_7: return "ORIGIN_1_DELTA_7";
    case ROGUE_IPF_COMPRESSION_FORMAT_RAW_BYTE: return "RAW_BYTE";
    case ROGUE_IPF_COMPRESSION_FORMAT_UNIQUE_2: return "UNIQUE_2";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_1: return "ORIGIN_2_DELTA_1";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_2: return "ORIGIN_2_DELTA_2";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_3: return "ORIGIN_2_DELTA_3";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_4: return "ORIGIN_2_DELTA_4";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_5: return "ORIGIN_2_DELTA_5";
    case ROGUE_IPF_COMPRESSION_FORMAT_ORIGIN_2_DELTA_6: return "ORIGIN_2_DELTA_6";
    default: return NULL;
    }
}

enum ROGUE_IPF_CS_MASK_FMT {
    ROGUE_IPF_CS_MASK_FMT_INDEX          =      0,
    ROGUE_IPF_CS_MASK_FMT_BYTE           =      1,
    ROGUE_IPF_CS_MASK_FMT_BIT            =      2,
    ROGUE_IPF_CS_MASK_FMT_FULL           =      3,
};

static __constant const char *
ROGUE_IPF_CS_MASK_FMT_to_str(const enum ROGUE_IPF_CS_MASK_FMT value)
{
    switch (value) {
    case ROGUE_IPF_CS_MASK_FMT_INDEX: return "INDEX";
    case ROGUE_IPF_CS_MASK_FMT_BYTE: return "BYTE";
    case ROGUE_IPF_CS_MASK_FMT_BIT: return "BIT";
    case ROGUE_IPF_CS_MASK_FMT_FULL: return "FULL";
    default: return NULL;
    }
}

enum ROGUE_IPF_CS_TYPE {
    ROGUE_IPF_CS_TYPE_PRIM               =      0,
    ROGUE_IPF_CS_TYPE_LINK               =      2,
    ROGUE_IPF_CS_TYPE_TERM               =      3,
};

static __constant const char *
ROGUE_IPF_CS_TYPE_to_str(const enum ROGUE_IPF_CS_TYPE value)
{
    switch (value) {
    case ROGUE_IPF_CS_TYPE_PRIM: return "PRIM";
    case ROGUE_IPF_CS_TYPE_LINK: return "LINK";
    case ROGUE_IPF_CS_TYPE_TERM: return "TERM";
    default: return NULL;
    }
}

enum ROGUE_IPF_CS_CTRL_TYPE_SIPF2 {
    ROGUE_IPF_CS_CTRL_TYPE_SIPF2_PRIM    =      0,
    ROGUE_IPF_CS_CTRL_TYPE_SIPF2_LINK    =      1,
    ROGUE_IPF_CS_CTRL_TYPE_SIPF2_TERM    =      3,
};

static __constant const char *
ROGUE_IPF_CS_CTRL_TYPE_SIPF2_to_str(const enum ROGUE_IPF_CS_CTRL_TYPE_SIPF2 value)
{
    switch (value) {
    case ROGUE_IPF_CS_CTRL_TYPE_SIPF2_PRIM: return "PRIM";
    case ROGUE_IPF_CS_CTRL_TYPE_SIPF2_LINK: return "LINK";
    case ROGUE_IPF_CS_CTRL_TYPE_SIPF2_TERM: return "TERM";
    default: return NULL;
    }
}

#define ROGUE_IPF_INDEX_DATA_WORDS_SIPF_length      2
#define ROGUE_IPF_INDEX_DATA_WORDS_SIPF_header  \


struct ROGUE_IPF_INDEX_DATA_WORDS_SIPF {
    bool                                 ix_triangle3_bf_flag;
    bool                                 ix_triangle3_edge_flag_ca;
    uint32_t                             ix_triangle3_index_2;
    bool                                 ix_triangle3_edge_flag_bc;
    uint32_t                             ix_triangle3_index_1;
    bool                                 ix_triangle3_edge_flag_ab;
    uint32_t                             ix_triangle3_index_0;
    bool                                 ix_triangle2_bf_flag;
    bool                                 ix_triangle2_edge_flag_ca;
    uint32_t                             ix_triangle2_index_2;
    bool                                 ix_triangle2_edge_flag_bc;
    uint32_t                             ix_triangle2_index_1;
    bool                                 ix_triangle2_edge_flag_ab;
    uint32_t                             ix_triangle2_index_0;
    bool                                 ix_triangle1_bf_flag;
    bool                                 ix_triangle1_edge_flag_ca;
    uint32_t                             ix_triangle1_index_2;
    bool                                 ix_triangle1_edge_flag_bc;
    uint32_t                             ix_triangle1_index_1;
    bool                                 ix_triangle1_edge_flag_ab;
    uint32_t                             ix_triangle1_index_0;
    bool                                 ix_triangle0_bf_flag;
    bool                                 ix_triangle0_edge_flag_ca;
    uint32_t                             ix_triangle0_index_2;
    bool                                 ix_triangle0_edge_flag_bc;
    uint32_t                             ix_triangle0_index_1;
    bool                                 ix_triangle0_edge_flag_ab;
    uint32_t                             ix_triangle0_index_0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_INDEX_DATA_WORDS_SIPF_pack(__attribute__((unused)) void * restrict dst,
                                     __attribute__((unused)) const struct ROGUE_IPF_INDEX_DATA_WORDS_SIPF * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->ix_triangle1_bf_flag, 31, 31) |
      __pvr_uint(values->ix_triangle1_edge_flag_ca, 30, 30) |
      __pvr_uint(values->ix_triangle1_index_2, 26, 29) |
      __pvr_uint(values->ix_triangle1_edge_flag_bc, 25, 25) |
      __pvr_uint(values->ix_triangle1_index_1, 21, 24) |
      __pvr_uint(values->ix_triangle1_edge_flag_ab, 20, 20) |
      __pvr_uint(values->ix_triangle1_index_0, 16, 19) |
      __pvr_uint(values->ix_triangle0_bf_flag, 15, 15) |
      __pvr_uint(values->ix_triangle0_edge_flag_ca, 14, 14) |
      __pvr_uint(values->ix_triangle0_index_2, 10, 13) |
      __pvr_uint(values->ix_triangle0_edge_flag_bc, 9, 9) |
      __pvr_uint(values->ix_triangle0_index_1, 5, 8) |
      __pvr_uint(values->ix_triangle0_edge_flag_ab, 4, 4) |
      __pvr_uint(values->ix_triangle0_index_0, 0, 3);

    dw[1] =
      __pvr_uint(values->ix_triangle3_bf_flag, 31, 31) |
      __pvr_uint(values->ix_triangle3_edge_flag_ca, 30, 30) |
      __pvr_uint(values->ix_triangle3_index_2, 26, 29) |
      __pvr_uint(values->ix_triangle3_edge_flag_bc, 25, 25) |
      __pvr_uint(values->ix_triangle3_index_1, 21, 24) |
      __pvr_uint(values->ix_triangle3_edge_flag_ab, 20, 20) |
      __pvr_uint(values->ix_triangle3_index_0, 16, 19) |
      __pvr_uint(values->ix_triangle2_bf_flag, 15, 15) |
      __pvr_uint(values->ix_triangle2_edge_flag_ca, 14, 14) |
      __pvr_uint(values->ix_triangle2_index_2, 10, 13) |
      __pvr_uint(values->ix_triangle2_edge_flag_bc, 9, 9) |
      __pvr_uint(values->ix_triangle2_index_1, 5, 8) |
      __pvr_uint(values->ix_triangle2_edge_flag_ab, 4, 4) |
      __pvr_uint(values->ix_triangle2_index_0, 0, 3);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_INDEX_DATA_WORDS_SIPF_unpack(__attribute__((unused)) const void * restrict src,
                                       __attribute__((unused)) struct ROGUE_IPF_INDEX_DATA_WORDS_SIPF * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->ix_triangle1_bf_flag = __pvr_uint_unpack(dw[0], 31, 31);
    values->ix_triangle1_edge_flag_ca = __pvr_uint_unpack(dw[0], 30, 30);
    values->ix_triangle1_index_2 = __pvr_uint_unpack(dw[0], 26, 29);
    values->ix_triangle1_edge_flag_bc = __pvr_uint_unpack(dw[0], 25, 25);
    values->ix_triangle1_index_1 = __pvr_uint_unpack(dw[0], 21, 24);
    values->ix_triangle1_edge_flag_ab = __pvr_uint_unpack(dw[0], 20, 20);
    values->ix_triangle1_index_0 = __pvr_uint_unpack(dw[0], 16, 19);
    values->ix_triangle0_bf_flag = __pvr_uint_unpack(dw[0], 15, 15);
    values->ix_triangle0_edge_flag_ca = __pvr_uint_unpack(dw[0], 14, 14);
    values->ix_triangle0_index_2 = __pvr_uint_unpack(dw[0], 10, 13);
    values->ix_triangle0_edge_flag_bc = __pvr_uint_unpack(dw[0], 9, 9);
    values->ix_triangle0_index_1 = __pvr_uint_unpack(dw[0], 5, 8);
    values->ix_triangle0_edge_flag_ab = __pvr_uint_unpack(dw[0], 4, 4);
    values->ix_triangle0_index_0 = __pvr_uint_unpack(dw[0], 0, 3);
    values->ix_triangle3_bf_flag = __pvr_uint_unpack(dw[1], 31, 31);
    values->ix_triangle3_edge_flag_ca = __pvr_uint_unpack(dw[1], 30, 30);
    values->ix_triangle3_index_2 = __pvr_uint_unpack(dw[1], 26, 29);
    values->ix_triangle3_edge_flag_bc = __pvr_uint_unpack(dw[1], 25, 25);
    values->ix_triangle3_index_1 = __pvr_uint_unpack(dw[1], 21, 24);
    values->ix_triangle3_edge_flag_ab = __pvr_uint_unpack(dw[1], 20, 20);
    values->ix_triangle3_index_0 = __pvr_uint_unpack(dw[1], 16, 19);
    values->ix_triangle2_bf_flag = __pvr_uint_unpack(dw[1], 15, 15);
    values->ix_triangle2_edge_flag_ca = __pvr_uint_unpack(dw[1], 14, 14);
    values->ix_triangle2_index_2 = __pvr_uint_unpack(dw[1], 10, 13);
    values->ix_triangle2_edge_flag_bc = __pvr_uint_unpack(dw[1], 9, 9);
    values->ix_triangle2_index_1 = __pvr_uint_unpack(dw[1], 5, 8);
    values->ix_triangle2_edge_flag_ab = __pvr_uint_unpack(dw[1], 4, 4);
    values->ix_triangle2_index_0 = __pvr_uint_unpack(dw[1], 0, 3);
}

#define ROGUE_IPF_VERTEX_FORMAT_WORD_SIPF2_length      2
#define ROGUE_IPF_VERTEX_FORMAT_WORD_SIPF2_header\


struct ROGUE_IPF_VERTEX_FORMAT_WORD_SIPF2 {
    uint32_t                             vf_isp_format_word;
    uint32_t                             vf_isp_state_size;
    bool                                 vf_tsp_vtx_raw;
    bool                                 vf_isp_vtx_raw;
    uint32_t                             vf_varying_total_16;
    uint32_t                             vf_varying_total_32;
    uint32_t                             vf_varying_vertex_bits;
    uint32_t                             vf_primitive_total;
    uint32_t                             vf_vertex_total;
    bool                                 vf_prim_msaa_disable;
    bool                                 vf_vertex_clipped;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_VERTEX_FORMAT_WORD_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                        __attribute__((unused)) const struct ROGUE_IPF_VERTEX_FORMAT_WORD_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->vf_varying_total_16, 30, 36) |
      __pvr_uint(values->vf_isp_format_word, 42, 63) |
      __pvr_uint(values->vf_isp_state_size, 39, 41) |
      __pvr_uint(values->vf_tsp_vtx_raw, 38, 38) |
      __pvr_uint(values->vf_isp_vtx_raw, 37, 37) |
      __pvr_uint(values->vf_varying_total_32, 23, 29) |
      __pvr_uint(values->vf_varying_vertex_bits, 11, 22) |
      __pvr_uint(values->vf_primitive_total, 6, 10) |
      __pvr_uint(values->vf_vertex_total, 2, 5) |
      __pvr_uint(values->vf_prim_msaa_disable, 1, 1) |
      __pvr_uint(values->vf_vertex_clipped, 0, 0);
    dw[0] = v0;
    dw[1] = v0 >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_VERTEX_FORMAT_WORD_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                          __attribute__((unused)) struct ROGUE_IPF_VERTEX_FORMAT_WORD_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->vf_varying_total_16 = __pvr_uint_unpack(v0, 30, 36);
    values->vf_isp_format_word = __pvr_uint_unpack(v0, 42, 63);
    values->vf_isp_state_size = __pvr_uint_unpack(v0, 39, 41);
    values->vf_tsp_vtx_raw = __pvr_uint_unpack(v0, 38, 38);
    values->vf_isp_vtx_raw = __pvr_uint_unpack(v0, 37, 37);
    values->vf_varying_total_32 = __pvr_uint_unpack(v0, 23, 29);
    values->vf_varying_vertex_bits = __pvr_uint_unpack(v0, 11, 22);
    values->vf_primitive_total = __pvr_uint_unpack(v0, 6, 10);
    values->vf_vertex_total = __pvr_uint_unpack(v0, 2, 5);
    values->vf_prim_msaa_disable = __pvr_uint_unpack(v0, 1, 1);
    values->vf_vertex_clipped = __pvr_uint_unpack(v0, 0, 0);
}

#define ROGUE_IPF_SCISSOR_WORD_0_length        1
#define ROGUE_IPF_SCISSOR_WORD_0_header         \


struct ROGUE_IPF_SCISSOR_WORD_0 {
    uint32_t                             scw0_xmin;
    uint32_t                             scw0_xmax;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_SCISSOR_WORD_0_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_IPF_SCISSOR_WORD_0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->scw0_xmin, 16, 31) |
      __pvr_uint(values->scw0_xmax, 0, 15);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_SCISSOR_WORD_0_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_IPF_SCISSOR_WORD_0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->scw0_xmin = __pvr_uint_unpack(dw[0], 16, 31);
    values->scw0_xmax = __pvr_uint_unpack(dw[0], 0, 15);
}

#define ROGUE_IPF_SCISSOR_WORD_1_length        1
#define ROGUE_IPF_SCISSOR_WORD_1_header         \


struct ROGUE_IPF_SCISSOR_WORD_1 {
    uint32_t                             scw1_ymin;
    uint32_t                             scw1_ymax;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_SCISSOR_WORD_1_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_IPF_SCISSOR_WORD_1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->scw1_ymin, 16, 31) |
      __pvr_uint(values->scw1_ymax, 0, 15);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_SCISSOR_WORD_1_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_IPF_SCISSOR_WORD_1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->scw1_ymin = __pvr_uint_unpack(dw[0], 16, 31);
    values->scw1_ymax = __pvr_uint_unpack(dw[0], 0, 15);
}

#define ROGUE_IPF_CONTROL_STREAM_length        1
#define ROGUE_IPF_CONTROL_STREAM_header         \


/* Helper macros for cs_link */
#define ROGUE_IPF_CONTROL_STREAM_CS_LINK_ALIGNMENT 128
#define ROGUE_IPF_CONTROL_STREAM_CS_LINK_UNIT_SIZE 128

struct ROGUE_IPF_CONTROL_STREAM {
    enum ROGUE_IPF_CS_TYPE               cs_type;
    __pvr_address_type                   cs_link;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_CONTROL_STREAM_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_IPF_CONTROL_STREAM * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint32_t v0 =
      __pvr_uint(values->cs_type, 30, 31);
    dw[0] = __pvr_address(values->cs_link, 7, 3, 29) | v0;
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_CONTROL_STREAM_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_IPF_CONTROL_STREAM * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_type = __pvr_uint_unpack(dw[0], 30, 31);
    values->cs_link = __pvr_address_unpack(dw[0], 7, 3, 29);
}

#define ROGUE_IPF_CONTROL_STREAM_LINK_SIPF2_length      1
#define ROGUE_IPF_CONTROL_STREAM_LINK_SIPF2_header\
    .cs_ctrl_type                        = ROGUE_IPF_CS_CTRL_TYPE_SIPF2_LINK

/* Helper macros for cs_link */
#define ROGUE_IPF_CONTROL_STREAM_LINK_SIPF2_CS_LINK_ALIGNMENT 64
#define ROGUE_IPF_CONTROL_STREAM_LINK_SIPF2_CS_LINK_UNIT_SIZE 64

struct ROGUE_IPF_CONTROL_STREAM_LINK_SIPF2 {
    __pvr_address_type                   cs_link;
    enum ROGUE_IPF_CS_CTRL_TYPE_SIPF2    cs_ctrl_type;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_CONTROL_STREAM_LINK_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                         __attribute__((unused)) const struct ROGUE_IPF_CONTROL_STREAM_LINK_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint32_t v0 =
      __pvr_uint(values->cs_ctrl_type, 0, 1);
    dw[0] = __pvr_address(values->cs_link, 6, 2, 31) | v0;
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_CONTROL_STREAM_LINK_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                           __attribute__((unused)) struct ROGUE_IPF_CONTROL_STREAM_LINK_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_link = __pvr_address_unpack(dw[0], 6, 2, 31);
    values->cs_ctrl_type = __pvr_uint_unpack(dw[0], 0, 1);
}

#define ROGUE_IPF_CONTROL_STREAM_TERMINATE_SIPF2_length      1
#define ROGUE_IPF_CONTROL_STREAM_TERMINATE_SIPF2_header\
    .cs_ctrl_type                        = ROGUE_IPF_CS_CTRL_TYPE_SIPF2_TERM

struct ROGUE_IPF_CONTROL_STREAM_TERMINATE_SIPF2 {
    uint32_t                             rsvd;
    enum ROGUE_IPF_CS_CTRL_TYPE_SIPF2    cs_ctrl_type;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_CONTROL_STREAM_TERMINATE_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                              __attribute__((unused)) const struct ROGUE_IPF_CONTROL_STREAM_TERMINATE_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->rsvd, 2, 7) |
      __pvr_uint(values->cs_ctrl_type, 0, 1);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_CONTROL_STREAM_TERMINATE_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                                __attribute__((unused)) struct ROGUE_IPF_CONTROL_STREAM_TERMINATE_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->rsvd = __pvr_uint_unpack(dw[0], 2, 7);
    values->cs_ctrl_type = __pvr_uint_unpack(dw[0], 0, 1);
}

#define ROGUE_IPF_PRIMITIVE_HEADER_SIPF2_length      1
#define ROGUE_IPF_PRIMITIVE_HEADER_SIPF2_header \
    .cs_ctrl_type                        = ROGUE_IPF_CS_CTRL_TYPE_SIPF2_PRIM

struct ROGUE_IPF_PRIMITIVE_HEADER_SIPF2 {
    bool                                 cs_valid_tile3;
    bool                                 cs_valid_tile2;
    bool                                 cs_valid_tile1;
    bool                                 cs_valid_tile0;
    uint32_t                             cs_mask_num_bytes;
    bool                                 cs_prim_base_size;
    enum ROGUE_IPF_CS_CTRL_TYPE_SIPF2    cs_ctrl_type;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_PRIMITIVE_HEADER_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                      __attribute__((unused)) const struct ROGUE_IPF_PRIMITIVE_HEADER_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_valid_tile3, 7, 7) |
      __pvr_uint(values->cs_valid_tile2, 6, 6) |
      __pvr_uint(values->cs_valid_tile1, 5, 5) |
      __pvr_uint(values->cs_valid_tile0, 4, 4) |
      __pvr_uint(values->cs_mask_num_bytes, 2, 3) |
      __pvr_uint(values->cs_prim_base_size, 1, 1) |
      __pvr_uint(values->cs_ctrl_type, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_PRIMITIVE_HEADER_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                        __attribute__((unused)) struct ROGUE_IPF_PRIMITIVE_HEADER_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_valid_tile3 = __pvr_uint_unpack(dw[0], 7, 7);
    values->cs_valid_tile2 = __pvr_uint_unpack(dw[0], 6, 6);
    values->cs_valid_tile1 = __pvr_uint_unpack(dw[0], 5, 5);
    values->cs_valid_tile0 = __pvr_uint_unpack(dw[0], 4, 4);
    values->cs_mask_num_bytes = __pvr_uint_unpack(dw[0], 2, 3);
    values->cs_prim_base_size = __pvr_uint_unpack(dw[0], 1, 1);
    values->cs_ctrl_type = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_IPF_PRIMITIVE_FORMAT_length      1
#define ROGUE_IPF_PRIMITIVE_FORMAT_header       \


struct ROGUE_IPF_PRIMITIVE_FORMAT {
    enum ROGUE_IPF_CS_TYPE               cs_type;
    bool                                 cs_isp_state_read;
    uint32_t                             cs_isp_state_size;
    uint32_t                             cs_prim_total;
    enum ROGUE_IPF_CS_MASK_FMT           cs_mask_fmt;
    bool                                 cs_prim_base_pres;
    uint32_t                             cs_prim_base_offset;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_PRIMITIVE_FORMAT_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_IPF_PRIMITIVE_FORMAT * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_type, 30, 31) |
      __pvr_uint(values->cs_isp_state_read, 29, 29) |
      __pvr_uint(values->cs_isp_state_size, 26, 28) |
      __pvr_uint(values->cs_prim_total, 19, 25) |
      __pvr_uint(values->cs_mask_fmt, 17, 18) |
      __pvr_uint(values->cs_prim_base_pres, 16, 16) |
      __pvr_uint(values->cs_prim_base_offset, 0, 15);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_PRIMITIVE_FORMAT_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_IPF_PRIMITIVE_FORMAT * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_type = __pvr_uint_unpack(dw[0], 30, 31);
    values->cs_isp_state_read = __pvr_uint_unpack(dw[0], 29, 29);
    values->cs_isp_state_size = __pvr_uint_unpack(dw[0], 26, 28);
    values->cs_prim_total = __pvr_uint_unpack(dw[0], 19, 25);
    values->cs_mask_fmt = __pvr_uint_unpack(dw[0], 17, 18);
    values->cs_prim_base_pres = __pvr_uint_unpack(dw[0], 16, 16);
    values->cs_prim_base_offset = __pvr_uint_unpack(dw[0], 0, 15);
}

#define ROGUE_IPF_PRIMITIVE_BASE_length        1
#define ROGUE_IPF_PRIMITIVE_BASE_header         \


/* Helper macros for cs_prim_base */
#define ROGUE_IPF_PRIMITIVE_BASE_CS_PRIM_BASE_ALIGNMENT 4

struct ROGUE_IPF_PRIMITIVE_BASE {
    __pvr_address_type                   cs_prim_base;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_PRIMITIVE_BASE_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_IPF_PRIMITIVE_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->cs_prim_base, 2, 0, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_PRIMITIVE_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_IPF_PRIMITIVE_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_prim_base = __pvr_address_unpack(dw[0], 2, 0, 31);
}

#define ROGUE_IPF_PRIMITIVE_BASE_SIPF2_length      1
#define ROGUE_IPF_PRIMITIVE_BASE_SIPF2_header   \


/* Helper macros for cs_prim_base */
#define ROGUE_IPF_PRIMITIVE_BASE_SIPF2_CS_PRIM_BASE_ALIGNMENT 8

struct ROGUE_IPF_PRIMITIVE_BASE_SIPF2 {
    __pvr_address_type                   cs_prim_base;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_PRIMITIVE_BASE_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                    __attribute__((unused)) const struct ROGUE_IPF_PRIMITIVE_BASE_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->cs_prim_base, 3, 0, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_PRIMITIVE_BASE_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                      __attribute__((unused)) struct ROGUE_IPF_PRIMITIVE_BASE_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_prim_base = __pvr_address_unpack(dw[0], 3, 0, 31);
}

#define ROGUE_IPF_BYTE_BASED_MASK_ONE_BYTE_WORD_0_SIPF2_length      1
#define ROGUE_IPF_BYTE_BASED_MASK_ONE_BYTE_WORD_0_SIPF2_header\


struct ROGUE_IPF_BYTE_BASED_MASK_ONE_BYTE_WORD_0_SIPF2 {
    bool                                 cs_mask_one_byte_tile3_7;
    bool                                 cs_mask_one_byte_tile3_6;
    bool                                 cs_mask_one_byte_tile3_5;
    bool                                 cs_mask_one_byte_tile3_4;
    bool                                 cs_mask_one_byte_tile3_3;
    bool                                 cs_mask_one_byte_tile3_2;
    bool                                 cs_mask_one_byte_tile3_1;
    bool                                 cs_mask_one_byte_tile3_0;
    bool                                 cs_mask_one_byte_tile2_7;
    bool                                 cs_mask_one_byte_tile2_6;
    bool                                 cs_mask_one_byte_tile2_5;
    bool                                 cs_mask_one_byte_tile2_4;
    bool                                 cs_mask_one_byte_tile2_3;
    bool                                 cs_mask_one_byte_tile2_2;
    bool                                 cs_mask_one_byte_tile2_1;
    bool                                 cs_mask_one_byte_tile2_0;
    bool                                 cs_mask_one_byte_tile1_7;
    bool                                 cs_mask_one_byte_tile1_6;
    bool                                 cs_mask_one_byte_tile1_5;
    bool                                 cs_mask_one_byte_tile1_4;
    bool                                 cs_mask_one_byte_tile1_3;
    bool                                 cs_mask_one_byte_tile1_2;
    bool                                 cs_mask_one_byte_tile1_1;
    bool                                 cs_mask_one_byte_tile1_0;
    bool                                 cs_mask_one_byte_tile0_7;
    bool                                 cs_mask_one_byte_tile0_6;
    bool                                 cs_mask_one_byte_tile0_5;
    bool                                 cs_mask_one_byte_tile0_4;
    bool                                 cs_mask_one_byte_tile0_3;
    bool                                 cs_mask_one_byte_tile0_2;
    bool                                 cs_mask_one_byte_tile0_1;
    bool                                 cs_mask_one_byte_tile0_0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_ONE_BYTE_WORD_0_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                                     __attribute__((unused)) const struct ROGUE_IPF_BYTE_BASED_MASK_ONE_BYTE_WORD_0_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_mask_one_byte_tile3_7, 31, 31) |
      __pvr_uint(values->cs_mask_one_byte_tile3_6, 30, 30) |
      __pvr_uint(values->cs_mask_one_byte_tile3_5, 29, 29) |
      __pvr_uint(values->cs_mask_one_byte_tile3_4, 28, 28) |
      __pvr_uint(values->cs_mask_one_byte_tile3_3, 27, 27) |
      __pvr_uint(values->cs_mask_one_byte_tile3_2, 26, 26) |
      __pvr_uint(values->cs_mask_one_byte_tile3_1, 25, 25) |
      __pvr_uint(values->cs_mask_one_byte_tile3_0, 24, 24) |
      __pvr_uint(values->cs_mask_one_byte_tile2_7, 23, 23) |
      __pvr_uint(values->cs_mask_one_byte_tile2_6, 22, 22) |
      __pvr_uint(values->cs_mask_one_byte_tile2_5, 21, 21) |
      __pvr_uint(values->cs_mask_one_byte_tile2_4, 20, 20) |
      __pvr_uint(values->cs_mask_one_byte_tile2_3, 19, 19) |
      __pvr_uint(values->cs_mask_one_byte_tile2_2, 18, 18) |
      __pvr_uint(values->cs_mask_one_byte_tile2_1, 17, 17) |
      __pvr_uint(values->cs_mask_one_byte_tile2_0, 16, 16) |
      __pvr_uint(values->cs_mask_one_byte_tile1_7, 15, 15) |
      __pvr_uint(values->cs_mask_one_byte_tile1_6, 14, 14) |
      __pvr_uint(values->cs_mask_one_byte_tile1_5, 13, 13) |
      __pvr_uint(values->cs_mask_one_byte_tile1_4, 12, 12) |
      __pvr_uint(values->cs_mask_one_byte_tile1_3, 11, 11) |
      __pvr_uint(values->cs_mask_one_byte_tile1_2, 10, 10) |
      __pvr_uint(values->cs_mask_one_byte_tile1_1, 9, 9) |
      __pvr_uint(values->cs_mask_one_byte_tile1_0, 8, 8) |
      __pvr_uint(values->cs_mask_one_byte_tile0_7, 7, 7) |
      __pvr_uint(values->cs_mask_one_byte_tile0_6, 6, 6) |
      __pvr_uint(values->cs_mask_one_byte_tile0_5, 5, 5) |
      __pvr_uint(values->cs_mask_one_byte_tile0_4, 4, 4) |
      __pvr_uint(values->cs_mask_one_byte_tile0_3, 3, 3) |
      __pvr_uint(values->cs_mask_one_byte_tile0_2, 2, 2) |
      __pvr_uint(values->cs_mask_one_byte_tile0_1, 1, 1) |
      __pvr_uint(values->cs_mask_one_byte_tile0_0, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_ONE_BYTE_WORD_0_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                                       __attribute__((unused)) struct ROGUE_IPF_BYTE_BASED_MASK_ONE_BYTE_WORD_0_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_mask_one_byte_tile3_7 = __pvr_uint_unpack(dw[0], 31, 31);
    values->cs_mask_one_byte_tile3_6 = __pvr_uint_unpack(dw[0], 30, 30);
    values->cs_mask_one_byte_tile3_5 = __pvr_uint_unpack(dw[0], 29, 29);
    values->cs_mask_one_byte_tile3_4 = __pvr_uint_unpack(dw[0], 28, 28);
    values->cs_mask_one_byte_tile3_3 = __pvr_uint_unpack(dw[0], 27, 27);
    values->cs_mask_one_byte_tile3_2 = __pvr_uint_unpack(dw[0], 26, 26);
    values->cs_mask_one_byte_tile3_1 = __pvr_uint_unpack(dw[0], 25, 25);
    values->cs_mask_one_byte_tile3_0 = __pvr_uint_unpack(dw[0], 24, 24);
    values->cs_mask_one_byte_tile2_7 = __pvr_uint_unpack(dw[0], 23, 23);
    values->cs_mask_one_byte_tile2_6 = __pvr_uint_unpack(dw[0], 22, 22);
    values->cs_mask_one_byte_tile2_5 = __pvr_uint_unpack(dw[0], 21, 21);
    values->cs_mask_one_byte_tile2_4 = __pvr_uint_unpack(dw[0], 20, 20);
    values->cs_mask_one_byte_tile2_3 = __pvr_uint_unpack(dw[0], 19, 19);
    values->cs_mask_one_byte_tile2_2 = __pvr_uint_unpack(dw[0], 18, 18);
    values->cs_mask_one_byte_tile2_1 = __pvr_uint_unpack(dw[0], 17, 17);
    values->cs_mask_one_byte_tile2_0 = __pvr_uint_unpack(dw[0], 16, 16);
    values->cs_mask_one_byte_tile1_7 = __pvr_uint_unpack(dw[0], 15, 15);
    values->cs_mask_one_byte_tile1_6 = __pvr_uint_unpack(dw[0], 14, 14);
    values->cs_mask_one_byte_tile1_5 = __pvr_uint_unpack(dw[0], 13, 13);
    values->cs_mask_one_byte_tile1_4 = __pvr_uint_unpack(dw[0], 12, 12);
    values->cs_mask_one_byte_tile1_3 = __pvr_uint_unpack(dw[0], 11, 11);
    values->cs_mask_one_byte_tile1_2 = __pvr_uint_unpack(dw[0], 10, 10);
    values->cs_mask_one_byte_tile1_1 = __pvr_uint_unpack(dw[0], 9, 9);
    values->cs_mask_one_byte_tile1_0 = __pvr_uint_unpack(dw[0], 8, 8);
    values->cs_mask_one_byte_tile0_7 = __pvr_uint_unpack(dw[0], 7, 7);
    values->cs_mask_one_byte_tile0_6 = __pvr_uint_unpack(dw[0], 6, 6);
    values->cs_mask_one_byte_tile0_5 = __pvr_uint_unpack(dw[0], 5, 5);
    values->cs_mask_one_byte_tile0_4 = __pvr_uint_unpack(dw[0], 4, 4);
    values->cs_mask_one_byte_tile0_3 = __pvr_uint_unpack(dw[0], 3, 3);
    values->cs_mask_one_byte_tile0_2 = __pvr_uint_unpack(dw[0], 2, 2);
    values->cs_mask_one_byte_tile0_1 = __pvr_uint_unpack(dw[0], 1, 1);
    values->cs_mask_one_byte_tile0_0 = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_0_SIPF2_length      1
#define ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_0_SIPF2_header\


struct ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_0_SIPF2 {
    bool                                 cs_mask_two_byte_tile1_15;
    bool                                 cs_mask_two_byte_tile1_14;
    bool                                 cs_mask_two_byte_tile1_13;
    bool                                 cs_mask_two_byte_tile1_12;
    bool                                 cs_mask_two_byte_tile1_11;
    bool                                 cs_mask_two_byte_tile1_10;
    bool                                 cs_mask_two_byte_tile1_9;
    bool                                 cs_mask_two_byte_tile1_8;
    bool                                 cs_mask_two_byte_tile1_7;
    bool                                 cs_mask_two_byte_tile1_6;
    bool                                 cs_mask_two_byte_tile1_5;
    bool                                 cs_mask_two_byte_tile1_4;
    bool                                 cs_mask_two_byte_tile1_3;
    bool                                 cs_mask_two_byte_tile1_2;
    bool                                 cs_mask_two_byte_tile1_1;
    bool                                 cs_mask_two_byte_tile1_0;
    bool                                 cs_mask_two_byte_tile0_15;
    bool                                 cs_mask_two_byte_tile0_14;
    bool                                 cs_mask_two_byte_tile0_13;
    bool                                 cs_mask_two_byte_tile0_12;
    bool                                 cs_mask_two_byte_tile0_11;
    bool                                 cs_mask_two_byte_tile0_10;
    bool                                 cs_mask_two_byte_tile0_9;
    bool                                 cs_mask_two_byte_tile0_8;
    bool                                 cs_mask_two_byte_tile0_7;
    bool                                 cs_mask_two_byte_tile0_6;
    bool                                 cs_mask_two_byte_tile0_5;
    bool                                 cs_mask_two_byte_tile0_4;
    bool                                 cs_mask_two_byte_tile0_3;
    bool                                 cs_mask_two_byte_tile0_2;
    bool                                 cs_mask_two_byte_tile0_1;
    bool                                 cs_mask_two_byte_tile0_0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_0_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                                     __attribute__((unused)) const struct ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_0_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_mask_two_byte_tile1_15, 31, 31) |
      __pvr_uint(values->cs_mask_two_byte_tile1_14, 30, 30) |
      __pvr_uint(values->cs_mask_two_byte_tile1_13, 29, 29) |
      __pvr_uint(values->cs_mask_two_byte_tile1_12, 28, 28) |
      __pvr_uint(values->cs_mask_two_byte_tile1_11, 27, 27) |
      __pvr_uint(values->cs_mask_two_byte_tile1_10, 26, 26) |
      __pvr_uint(values->cs_mask_two_byte_tile1_9, 25, 25) |
      __pvr_uint(values->cs_mask_two_byte_tile1_8, 24, 24) |
      __pvr_uint(values->cs_mask_two_byte_tile1_7, 23, 23) |
      __pvr_uint(values->cs_mask_two_byte_tile1_6, 22, 22) |
      __pvr_uint(values->cs_mask_two_byte_tile1_5, 21, 21) |
      __pvr_uint(values->cs_mask_two_byte_tile1_4, 20, 20) |
      __pvr_uint(values->cs_mask_two_byte_tile1_3, 19, 19) |
      __pvr_uint(values->cs_mask_two_byte_tile1_2, 18, 18) |
      __pvr_uint(values->cs_mask_two_byte_tile1_1, 17, 17) |
      __pvr_uint(values->cs_mask_two_byte_tile1_0, 16, 16) |
      __pvr_uint(values->cs_mask_two_byte_tile0_15, 15, 15) |
      __pvr_uint(values->cs_mask_two_byte_tile0_14, 14, 14) |
      __pvr_uint(values->cs_mask_two_byte_tile0_13, 13, 13) |
      __pvr_uint(values->cs_mask_two_byte_tile0_12, 12, 12) |
      __pvr_uint(values->cs_mask_two_byte_tile0_11, 11, 11) |
      __pvr_uint(values->cs_mask_two_byte_tile0_10, 10, 10) |
      __pvr_uint(values->cs_mask_two_byte_tile0_9, 9, 9) |
      __pvr_uint(values->cs_mask_two_byte_tile0_8, 8, 8) |
      __pvr_uint(values->cs_mask_two_byte_tile0_7, 7, 7) |
      __pvr_uint(values->cs_mask_two_byte_tile0_6, 6, 6) |
      __pvr_uint(values->cs_mask_two_byte_tile0_5, 5, 5) |
      __pvr_uint(values->cs_mask_two_byte_tile0_4, 4, 4) |
      __pvr_uint(values->cs_mask_two_byte_tile0_3, 3, 3) |
      __pvr_uint(values->cs_mask_two_byte_tile0_2, 2, 2) |
      __pvr_uint(values->cs_mask_two_byte_tile0_1, 1, 1) |
      __pvr_uint(values->cs_mask_two_byte_tile0_0, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_0_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                                       __attribute__((unused)) struct ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_0_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_mask_two_byte_tile1_15 = __pvr_uint_unpack(dw[0], 31, 31);
    values->cs_mask_two_byte_tile1_14 = __pvr_uint_unpack(dw[0], 30, 30);
    values->cs_mask_two_byte_tile1_13 = __pvr_uint_unpack(dw[0], 29, 29);
    values->cs_mask_two_byte_tile1_12 = __pvr_uint_unpack(dw[0], 28, 28);
    values->cs_mask_two_byte_tile1_11 = __pvr_uint_unpack(dw[0], 27, 27);
    values->cs_mask_two_byte_tile1_10 = __pvr_uint_unpack(dw[0], 26, 26);
    values->cs_mask_two_byte_tile1_9 = __pvr_uint_unpack(dw[0], 25, 25);
    values->cs_mask_two_byte_tile1_8 = __pvr_uint_unpack(dw[0], 24, 24);
    values->cs_mask_two_byte_tile1_7 = __pvr_uint_unpack(dw[0], 23, 23);
    values->cs_mask_two_byte_tile1_6 = __pvr_uint_unpack(dw[0], 22, 22);
    values->cs_mask_two_byte_tile1_5 = __pvr_uint_unpack(dw[0], 21, 21);
    values->cs_mask_two_byte_tile1_4 = __pvr_uint_unpack(dw[0], 20, 20);
    values->cs_mask_two_byte_tile1_3 = __pvr_uint_unpack(dw[0], 19, 19);
    values->cs_mask_two_byte_tile1_2 = __pvr_uint_unpack(dw[0], 18, 18);
    values->cs_mask_two_byte_tile1_1 = __pvr_uint_unpack(dw[0], 17, 17);
    values->cs_mask_two_byte_tile1_0 = __pvr_uint_unpack(dw[0], 16, 16);
    values->cs_mask_two_byte_tile0_15 = __pvr_uint_unpack(dw[0], 15, 15);
    values->cs_mask_two_byte_tile0_14 = __pvr_uint_unpack(dw[0], 14, 14);
    values->cs_mask_two_byte_tile0_13 = __pvr_uint_unpack(dw[0], 13, 13);
    values->cs_mask_two_byte_tile0_12 = __pvr_uint_unpack(dw[0], 12, 12);
    values->cs_mask_two_byte_tile0_11 = __pvr_uint_unpack(dw[0], 11, 11);
    values->cs_mask_two_byte_tile0_10 = __pvr_uint_unpack(dw[0], 10, 10);
    values->cs_mask_two_byte_tile0_9 = __pvr_uint_unpack(dw[0], 9, 9);
    values->cs_mask_two_byte_tile0_8 = __pvr_uint_unpack(dw[0], 8, 8);
    values->cs_mask_two_byte_tile0_7 = __pvr_uint_unpack(dw[0], 7, 7);
    values->cs_mask_two_byte_tile0_6 = __pvr_uint_unpack(dw[0], 6, 6);
    values->cs_mask_two_byte_tile0_5 = __pvr_uint_unpack(dw[0], 5, 5);
    values->cs_mask_two_byte_tile0_4 = __pvr_uint_unpack(dw[0], 4, 4);
    values->cs_mask_two_byte_tile0_3 = __pvr_uint_unpack(dw[0], 3, 3);
    values->cs_mask_two_byte_tile0_2 = __pvr_uint_unpack(dw[0], 2, 2);
    values->cs_mask_two_byte_tile0_1 = __pvr_uint_unpack(dw[0], 1, 1);
    values->cs_mask_two_byte_tile0_0 = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_1_SIPF2_length      1
#define ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_1_SIPF2_header\


struct ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_1_SIPF2 {
    bool                                 cs_mask_two_byte_tile3_15;
    bool                                 cs_mask_two_byte_tile3_14;
    bool                                 cs_mask_two_byte_tile3_13;
    bool                                 cs_mask_two_byte_tile3_12;
    bool                                 cs_mask_two_byte_tile3_11;
    bool                                 cs_mask_two_byte_tile3_10;
    bool                                 cs_mask_two_byte_tile3_9;
    bool                                 cs_mask_two_byte_tile3_8;
    bool                                 cs_mask_two_byte_tile3_7;
    bool                                 cs_mask_two_byte_tile3_6;
    bool                                 cs_mask_two_byte_tile3_5;
    bool                                 cs_mask_two_byte_tile3_4;
    bool                                 cs_mask_two_byte_tile3_3;
    bool                                 cs_mask_two_byte_tile3_2;
    bool                                 cs_mask_two_byte_tile3_1;
    bool                                 cs_mask_two_byte_tile3_0;
    bool                                 cs_mask_two_byte_tile2_15;
    bool                                 cs_mask_two_byte_tile2_14;
    bool                                 cs_mask_two_byte_tile2_13;
    bool                                 cs_mask_two_byte_tile2_12;
    bool                                 cs_mask_two_byte_tile2_11;
    bool                                 cs_mask_two_byte_tile2_10;
    bool                                 cs_mask_two_byte_tile2_9;
    bool                                 cs_mask_two_byte_tile2_8;
    bool                                 cs_mask_two_byte_tile2_7;
    bool                                 cs_mask_two_byte_tile2_6;
    bool                                 cs_mask_two_byte_tile2_5;
    bool                                 cs_mask_two_byte_tile2_4;
    bool                                 cs_mask_two_byte_tile2_3;
    bool                                 cs_mask_two_byte_tile2_2;
    bool                                 cs_mask_two_byte_tile2_1;
    bool                                 cs_mask_two_byte_tile2_0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_1_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                                     __attribute__((unused)) const struct ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_1_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_mask_two_byte_tile3_15, 31, 31) |
      __pvr_uint(values->cs_mask_two_byte_tile3_14, 30, 30) |
      __pvr_uint(values->cs_mask_two_byte_tile3_13, 29, 29) |
      __pvr_uint(values->cs_mask_two_byte_tile3_12, 28, 28) |
      __pvr_uint(values->cs_mask_two_byte_tile3_11, 27, 27) |
      __pvr_uint(values->cs_mask_two_byte_tile3_10, 26, 26) |
      __pvr_uint(values->cs_mask_two_byte_tile3_9, 25, 25) |
      __pvr_uint(values->cs_mask_two_byte_tile3_8, 24, 24) |
      __pvr_uint(values->cs_mask_two_byte_tile3_7, 23, 23) |
      __pvr_uint(values->cs_mask_two_byte_tile3_6, 22, 22) |
      __pvr_uint(values->cs_mask_two_byte_tile3_5, 21, 21) |
      __pvr_uint(values->cs_mask_two_byte_tile3_4, 20, 20) |
      __pvr_uint(values->cs_mask_two_byte_tile3_3, 19, 19) |
      __pvr_uint(values->cs_mask_two_byte_tile3_2, 18, 18) |
      __pvr_uint(values->cs_mask_two_byte_tile3_1, 17, 17) |
      __pvr_uint(values->cs_mask_two_byte_tile3_0, 16, 16) |
      __pvr_uint(values->cs_mask_two_byte_tile2_15, 15, 15) |
      __pvr_uint(values->cs_mask_two_byte_tile2_14, 14, 14) |
      __pvr_uint(values->cs_mask_two_byte_tile2_13, 13, 13) |
      __pvr_uint(values->cs_mask_two_byte_tile2_12, 12, 12) |
      __pvr_uint(values->cs_mask_two_byte_tile2_11, 11, 11) |
      __pvr_uint(values->cs_mask_two_byte_tile2_10, 10, 10) |
      __pvr_uint(values->cs_mask_two_byte_tile2_9, 9, 9) |
      __pvr_uint(values->cs_mask_two_byte_tile2_8, 8, 8) |
      __pvr_uint(values->cs_mask_two_byte_tile2_7, 7, 7) |
      __pvr_uint(values->cs_mask_two_byte_tile2_6, 6, 6) |
      __pvr_uint(values->cs_mask_two_byte_tile2_5, 5, 5) |
      __pvr_uint(values->cs_mask_two_byte_tile2_4, 4, 4) |
      __pvr_uint(values->cs_mask_two_byte_tile2_3, 3, 3) |
      __pvr_uint(values->cs_mask_two_byte_tile2_2, 2, 2) |
      __pvr_uint(values->cs_mask_two_byte_tile2_1, 1, 1) |
      __pvr_uint(values->cs_mask_two_byte_tile2_0, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_1_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                                       __attribute__((unused)) struct ROGUE_IPF_BYTE_BASED_MASK_TWO_BYTE_WORD_1_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_mask_two_byte_tile3_15 = __pvr_uint_unpack(dw[0], 31, 31);
    values->cs_mask_two_byte_tile3_14 = __pvr_uint_unpack(dw[0], 30, 30);
    values->cs_mask_two_byte_tile3_13 = __pvr_uint_unpack(dw[0], 29, 29);
    values->cs_mask_two_byte_tile3_12 = __pvr_uint_unpack(dw[0], 28, 28);
    values->cs_mask_two_byte_tile3_11 = __pvr_uint_unpack(dw[0], 27, 27);
    values->cs_mask_two_byte_tile3_10 = __pvr_uint_unpack(dw[0], 26, 26);
    values->cs_mask_two_byte_tile3_9 = __pvr_uint_unpack(dw[0], 25, 25);
    values->cs_mask_two_byte_tile3_8 = __pvr_uint_unpack(dw[0], 24, 24);
    values->cs_mask_two_byte_tile3_7 = __pvr_uint_unpack(dw[0], 23, 23);
    values->cs_mask_two_byte_tile3_6 = __pvr_uint_unpack(dw[0], 22, 22);
    values->cs_mask_two_byte_tile3_5 = __pvr_uint_unpack(dw[0], 21, 21);
    values->cs_mask_two_byte_tile3_4 = __pvr_uint_unpack(dw[0], 20, 20);
    values->cs_mask_two_byte_tile3_3 = __pvr_uint_unpack(dw[0], 19, 19);
    values->cs_mask_two_byte_tile3_2 = __pvr_uint_unpack(dw[0], 18, 18);
    values->cs_mask_two_byte_tile3_1 = __pvr_uint_unpack(dw[0], 17, 17);
    values->cs_mask_two_byte_tile3_0 = __pvr_uint_unpack(dw[0], 16, 16);
    values->cs_mask_two_byte_tile2_15 = __pvr_uint_unpack(dw[0], 15, 15);
    values->cs_mask_two_byte_tile2_14 = __pvr_uint_unpack(dw[0], 14, 14);
    values->cs_mask_two_byte_tile2_13 = __pvr_uint_unpack(dw[0], 13, 13);
    values->cs_mask_two_byte_tile2_12 = __pvr_uint_unpack(dw[0], 12, 12);
    values->cs_mask_two_byte_tile2_11 = __pvr_uint_unpack(dw[0], 11, 11);
    values->cs_mask_two_byte_tile2_10 = __pvr_uint_unpack(dw[0], 10, 10);
    values->cs_mask_two_byte_tile2_9 = __pvr_uint_unpack(dw[0], 9, 9);
    values->cs_mask_two_byte_tile2_8 = __pvr_uint_unpack(dw[0], 8, 8);
    values->cs_mask_two_byte_tile2_7 = __pvr_uint_unpack(dw[0], 7, 7);
    values->cs_mask_two_byte_tile2_6 = __pvr_uint_unpack(dw[0], 6, 6);
    values->cs_mask_two_byte_tile2_5 = __pvr_uint_unpack(dw[0], 5, 5);
    values->cs_mask_two_byte_tile2_4 = __pvr_uint_unpack(dw[0], 4, 4);
    values->cs_mask_two_byte_tile2_3 = __pvr_uint_unpack(dw[0], 3, 3);
    values->cs_mask_two_byte_tile2_2 = __pvr_uint_unpack(dw[0], 2, 2);
    values->cs_mask_two_byte_tile2_1 = __pvr_uint_unpack(dw[0], 1, 1);
    values->cs_mask_two_byte_tile2_0 = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_0_SIPF2_length      1
#define ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_0_SIPF2_header\


struct ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_0_SIPF2 {
    bool                                 cs_mask_three_byte_tile1_7;
    bool                                 cs_mask_three_byte_tile1_6;
    bool                                 cs_mask_three_byte_tile1_5;
    bool                                 cs_mask_three_byte_tile1_4;
    bool                                 cs_mask_three_byte_tile1_3;
    bool                                 cs_mask_three_byte_tile1_2;
    bool                                 cs_mask_three_byte_tile1_1;
    bool                                 cs_mask_three_byte_tile1_0;
    bool                                 cs_mask_three_byte_tile0_23;
    bool                                 cs_mask_three_byte_tile0_22;
    bool                                 cs_mask_three_byte_tile0_21;
    bool                                 cs_mask_three_byte_tile0_20;
    bool                                 cs_mask_three_byte_tile0_19;
    bool                                 cs_mask_three_byte_tile0_18;
    bool                                 cs_mask_three_byte_tile0_17;
    bool                                 cs_mask_three_byte_tile0_16;
    bool                                 cs_mask_three_byte_tile0_15;
    bool                                 cs_mask_three_byte_tile0_14;
    bool                                 cs_mask_three_byte_tile0_13;
    bool                                 cs_mask_three_byte_tile0_12;
    bool                                 cs_mask_three_byte_tile0_11;
    bool                                 cs_mask_three_byte_tile0_10;
    bool                                 cs_mask_three_byte_tile0_9;
    bool                                 cs_mask_three_byte_tile0_8;
    bool                                 cs_mask_three_byte_tile0_7;
    bool                                 cs_mask_three_byte_tile0_6;
    bool                                 cs_mask_three_byte_tile0_5;
    bool                                 cs_mask_three_byte_tile0_4;
    bool                                 cs_mask_three_byte_tile0_3;
    bool                                 cs_mask_three_byte_tile0_2;
    bool                                 cs_mask_three_byte_tile0_1;
    bool                                 cs_mask_three_byte_tile0_0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_0_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                                       __attribute__((unused)) const struct ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_0_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_mask_three_byte_tile1_7, 31, 31) |
      __pvr_uint(values->cs_mask_three_byte_tile1_6, 30, 30) |
      __pvr_uint(values->cs_mask_three_byte_tile1_5, 29, 29) |
      __pvr_uint(values->cs_mask_three_byte_tile1_4, 28, 28) |
      __pvr_uint(values->cs_mask_three_byte_tile1_3, 27, 27) |
      __pvr_uint(values->cs_mask_three_byte_tile1_2, 26, 26) |
      __pvr_uint(values->cs_mask_three_byte_tile1_1, 25, 25) |
      __pvr_uint(values->cs_mask_three_byte_tile1_0, 24, 24) |
      __pvr_uint(values->cs_mask_three_byte_tile0_23, 23, 23) |
      __pvr_uint(values->cs_mask_three_byte_tile0_22, 22, 22) |
      __pvr_uint(values->cs_mask_three_byte_tile0_21, 21, 21) |
      __pvr_uint(values->cs_mask_three_byte_tile0_20, 20, 20) |
      __pvr_uint(values->cs_mask_three_byte_tile0_19, 19, 19) |
      __pvr_uint(values->cs_mask_three_byte_tile0_18, 18, 18) |
      __pvr_uint(values->cs_mask_three_byte_tile0_17, 17, 17) |
      __pvr_uint(values->cs_mask_three_byte_tile0_16, 16, 16) |
      __pvr_uint(values->cs_mask_three_byte_tile0_15, 15, 15) |
      __pvr_uint(values->cs_mask_three_byte_tile0_14, 14, 14) |
      __pvr_uint(values->cs_mask_three_byte_tile0_13, 13, 13) |
      __pvr_uint(values->cs_mask_three_byte_tile0_12, 12, 12) |
      __pvr_uint(values->cs_mask_three_byte_tile0_11, 11, 11) |
      __pvr_uint(values->cs_mask_three_byte_tile0_10, 10, 10) |
      __pvr_uint(values->cs_mask_three_byte_tile0_9, 9, 9) |
      __pvr_uint(values->cs_mask_three_byte_tile0_8, 8, 8) |
      __pvr_uint(values->cs_mask_three_byte_tile0_7, 7, 7) |
      __pvr_uint(values->cs_mask_three_byte_tile0_6, 6, 6) |
      __pvr_uint(values->cs_mask_three_byte_tile0_5, 5, 5) |
      __pvr_uint(values->cs_mask_three_byte_tile0_4, 4, 4) |
      __pvr_uint(values->cs_mask_three_byte_tile0_3, 3, 3) |
      __pvr_uint(values->cs_mask_three_byte_tile0_2, 2, 2) |
      __pvr_uint(values->cs_mask_three_byte_tile0_1, 1, 1) |
      __pvr_uint(values->cs_mask_three_byte_tile0_0, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_0_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                                         __attribute__((unused)) struct ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_0_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_mask_three_byte_tile1_7 = __pvr_uint_unpack(dw[0], 31, 31);
    values->cs_mask_three_byte_tile1_6 = __pvr_uint_unpack(dw[0], 30, 30);
    values->cs_mask_three_byte_tile1_5 = __pvr_uint_unpack(dw[0], 29, 29);
    values->cs_mask_three_byte_tile1_4 = __pvr_uint_unpack(dw[0], 28, 28);
    values->cs_mask_three_byte_tile1_3 = __pvr_uint_unpack(dw[0], 27, 27);
    values->cs_mask_three_byte_tile1_2 = __pvr_uint_unpack(dw[0], 26, 26);
    values->cs_mask_three_byte_tile1_1 = __pvr_uint_unpack(dw[0], 25, 25);
    values->cs_mask_three_byte_tile1_0 = __pvr_uint_unpack(dw[0], 24, 24);
    values->cs_mask_three_byte_tile0_23 = __pvr_uint_unpack(dw[0], 23, 23);
    values->cs_mask_three_byte_tile0_22 = __pvr_uint_unpack(dw[0], 22, 22);
    values->cs_mask_three_byte_tile0_21 = __pvr_uint_unpack(dw[0], 21, 21);
    values->cs_mask_three_byte_tile0_20 = __pvr_uint_unpack(dw[0], 20, 20);
    values->cs_mask_three_byte_tile0_19 = __pvr_uint_unpack(dw[0], 19, 19);
    values->cs_mask_three_byte_tile0_18 = __pvr_uint_unpack(dw[0], 18, 18);
    values->cs_mask_three_byte_tile0_17 = __pvr_uint_unpack(dw[0], 17, 17);
    values->cs_mask_three_byte_tile0_16 = __pvr_uint_unpack(dw[0], 16, 16);
    values->cs_mask_three_byte_tile0_15 = __pvr_uint_unpack(dw[0], 15, 15);
    values->cs_mask_three_byte_tile0_14 = __pvr_uint_unpack(dw[0], 14, 14);
    values->cs_mask_three_byte_tile0_13 = __pvr_uint_unpack(dw[0], 13, 13);
    values->cs_mask_three_byte_tile0_12 = __pvr_uint_unpack(dw[0], 12, 12);
    values->cs_mask_three_byte_tile0_11 = __pvr_uint_unpack(dw[0], 11, 11);
    values->cs_mask_three_byte_tile0_10 = __pvr_uint_unpack(dw[0], 10, 10);
    values->cs_mask_three_byte_tile0_9 = __pvr_uint_unpack(dw[0], 9, 9);
    values->cs_mask_three_byte_tile0_8 = __pvr_uint_unpack(dw[0], 8, 8);
    values->cs_mask_three_byte_tile0_7 = __pvr_uint_unpack(dw[0], 7, 7);
    values->cs_mask_three_byte_tile0_6 = __pvr_uint_unpack(dw[0], 6, 6);
    values->cs_mask_three_byte_tile0_5 = __pvr_uint_unpack(dw[0], 5, 5);
    values->cs_mask_three_byte_tile0_4 = __pvr_uint_unpack(dw[0], 4, 4);
    values->cs_mask_three_byte_tile0_3 = __pvr_uint_unpack(dw[0], 3, 3);
    values->cs_mask_three_byte_tile0_2 = __pvr_uint_unpack(dw[0], 2, 2);
    values->cs_mask_three_byte_tile0_1 = __pvr_uint_unpack(dw[0], 1, 1);
    values->cs_mask_three_byte_tile0_0 = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_1_SIPF2_length      1
#define ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_1_SIPF2_header\


struct ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_1_SIPF2 {
    bool                                 cs_mask_three_byte_tile2_15;
    bool                                 cs_mask_three_byte_tile2_14;
    bool                                 cs_mask_three_byte_tile2_13;
    bool                                 cs_mask_three_byte_tile2_12;
    bool                                 cs_mask_three_byte_tile2_11;
    bool                                 cs_mask_three_byte_tile2_10;
    bool                                 cs_mask_three_byte_tile2_9;
    bool                                 cs_mask_three_byte_tile2_8;
    bool                                 cs_mask_three_byte_tile2_7;
    bool                                 cs_mask_three_byte_tile2_6;
    bool                                 cs_mask_three_byte_tile2_5;
    bool                                 cs_mask_three_byte_tile2_4;
    bool                                 cs_mask_three_byte_tile2_3;
    bool                                 cs_mask_three_byte_tile2_2;
    bool                                 cs_mask_three_byte_tile2_1;
    bool                                 cs_mask_three_byte_tile2_0;
    bool                                 cs_mask_three_byte_tile1_23;
    bool                                 cs_mask_three_byte_tile1_22;
    bool                                 cs_mask_three_byte_tile1_21;
    bool                                 cs_mask_three_byte_tile1_20;
    bool                                 cs_mask_three_byte_tile1_19;
    bool                                 cs_mask_three_byte_tile1_18;
    bool                                 cs_mask_three_byte_tile1_17;
    bool                                 cs_mask_three_byte_tile1_16;
    bool                                 cs_mask_three_byte_tile1_15;
    bool                                 cs_mask_three_byte_tile1_14;
    bool                                 cs_mask_three_byte_tile1_13;
    bool                                 cs_mask_three_byte_tile1_12;
    bool                                 cs_mask_three_byte_tile1_11;
    bool                                 cs_mask_three_byte_tile1_10;
    bool                                 cs_mask_three_byte_tile1_9;
    bool                                 cs_mask_three_byte_tile1_8;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_1_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                                       __attribute__((unused)) const struct ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_1_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_mask_three_byte_tile2_15, 31, 31) |
      __pvr_uint(values->cs_mask_three_byte_tile2_14, 30, 30) |
      __pvr_uint(values->cs_mask_three_byte_tile2_13, 29, 29) |
      __pvr_uint(values->cs_mask_three_byte_tile2_12, 28, 28) |
      __pvr_uint(values->cs_mask_three_byte_tile2_11, 27, 27) |
      __pvr_uint(values->cs_mask_three_byte_tile2_10, 26, 26) |
      __pvr_uint(values->cs_mask_three_byte_tile2_9, 25, 25) |
      __pvr_uint(values->cs_mask_three_byte_tile2_8, 24, 24) |
      __pvr_uint(values->cs_mask_three_byte_tile2_7, 23, 23) |
      __pvr_uint(values->cs_mask_three_byte_tile2_6, 22, 22) |
      __pvr_uint(values->cs_mask_three_byte_tile2_5, 21, 21) |
      __pvr_uint(values->cs_mask_three_byte_tile2_4, 20, 20) |
      __pvr_uint(values->cs_mask_three_byte_tile2_3, 19, 19) |
      __pvr_uint(values->cs_mask_three_byte_tile2_2, 18, 18) |
      __pvr_uint(values->cs_mask_three_byte_tile2_1, 17, 17) |
      __pvr_uint(values->cs_mask_three_byte_tile2_0, 16, 16) |
      __pvr_uint(values->cs_mask_three_byte_tile1_23, 15, 15) |
      __pvr_uint(values->cs_mask_three_byte_tile1_22, 14, 14) |
      __pvr_uint(values->cs_mask_three_byte_tile1_21, 13, 13) |
      __pvr_uint(values->cs_mask_three_byte_tile1_20, 12, 12) |
      __pvr_uint(values->cs_mask_three_byte_tile1_19, 11, 11) |
      __pvr_uint(values->cs_mask_three_byte_tile1_18, 10, 10) |
      __pvr_uint(values->cs_mask_three_byte_tile1_17, 9, 9) |
      __pvr_uint(values->cs_mask_three_byte_tile1_16, 8, 8) |
      __pvr_uint(values->cs_mask_three_byte_tile1_15, 7, 7) |
      __pvr_uint(values->cs_mask_three_byte_tile1_14, 6, 6) |
      __pvr_uint(values->cs_mask_three_byte_tile1_13, 5, 5) |
      __pvr_uint(values->cs_mask_three_byte_tile1_12, 4, 4) |
      __pvr_uint(values->cs_mask_three_byte_tile1_11, 3, 3) |
      __pvr_uint(values->cs_mask_three_byte_tile1_10, 2, 2) |
      __pvr_uint(values->cs_mask_three_byte_tile1_9, 1, 1) |
      __pvr_uint(values->cs_mask_three_byte_tile1_8, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_1_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                                         __attribute__((unused)) struct ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_1_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_mask_three_byte_tile2_15 = __pvr_uint_unpack(dw[0], 31, 31);
    values->cs_mask_three_byte_tile2_14 = __pvr_uint_unpack(dw[0], 30, 30);
    values->cs_mask_three_byte_tile2_13 = __pvr_uint_unpack(dw[0], 29, 29);
    values->cs_mask_three_byte_tile2_12 = __pvr_uint_unpack(dw[0], 28, 28);
    values->cs_mask_three_byte_tile2_11 = __pvr_uint_unpack(dw[0], 27, 27);
    values->cs_mask_three_byte_tile2_10 = __pvr_uint_unpack(dw[0], 26, 26);
    values->cs_mask_three_byte_tile2_9 = __pvr_uint_unpack(dw[0], 25, 25);
    values->cs_mask_three_byte_tile2_8 = __pvr_uint_unpack(dw[0], 24, 24);
    values->cs_mask_three_byte_tile2_7 = __pvr_uint_unpack(dw[0], 23, 23);
    values->cs_mask_three_byte_tile2_6 = __pvr_uint_unpack(dw[0], 22, 22);
    values->cs_mask_three_byte_tile2_5 = __pvr_uint_unpack(dw[0], 21, 21);
    values->cs_mask_three_byte_tile2_4 = __pvr_uint_unpack(dw[0], 20, 20);
    values->cs_mask_three_byte_tile2_3 = __pvr_uint_unpack(dw[0], 19, 19);
    values->cs_mask_three_byte_tile2_2 = __pvr_uint_unpack(dw[0], 18, 18);
    values->cs_mask_three_byte_tile2_1 = __pvr_uint_unpack(dw[0], 17, 17);
    values->cs_mask_three_byte_tile2_0 = __pvr_uint_unpack(dw[0], 16, 16);
    values->cs_mask_three_byte_tile1_23 = __pvr_uint_unpack(dw[0], 15, 15);
    values->cs_mask_three_byte_tile1_22 = __pvr_uint_unpack(dw[0], 14, 14);
    values->cs_mask_three_byte_tile1_21 = __pvr_uint_unpack(dw[0], 13, 13);
    values->cs_mask_three_byte_tile1_20 = __pvr_uint_unpack(dw[0], 12, 12);
    values->cs_mask_three_byte_tile1_19 = __pvr_uint_unpack(dw[0], 11, 11);
    values->cs_mask_three_byte_tile1_18 = __pvr_uint_unpack(dw[0], 10, 10);
    values->cs_mask_three_byte_tile1_17 = __pvr_uint_unpack(dw[0], 9, 9);
    values->cs_mask_three_byte_tile1_16 = __pvr_uint_unpack(dw[0], 8, 8);
    values->cs_mask_three_byte_tile1_15 = __pvr_uint_unpack(dw[0], 7, 7);
    values->cs_mask_three_byte_tile1_14 = __pvr_uint_unpack(dw[0], 6, 6);
    values->cs_mask_three_byte_tile1_13 = __pvr_uint_unpack(dw[0], 5, 5);
    values->cs_mask_three_byte_tile1_12 = __pvr_uint_unpack(dw[0], 4, 4);
    values->cs_mask_three_byte_tile1_11 = __pvr_uint_unpack(dw[0], 3, 3);
    values->cs_mask_three_byte_tile1_10 = __pvr_uint_unpack(dw[0], 2, 2);
    values->cs_mask_three_byte_tile1_9 = __pvr_uint_unpack(dw[0], 1, 1);
    values->cs_mask_three_byte_tile1_8 = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_2_SIPF2_length      1
#define ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_2_SIPF2_header\


struct ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_2_SIPF2 {
    bool                                 cs_mask_three_byte_tile3_23;
    bool                                 cs_mask_three_byte_tile3_22;
    bool                                 cs_mask_three_byte_tile3_21;
    bool                                 cs_mask_three_byte_tile3_20;
    bool                                 cs_mask_three_byte_tile3_19;
    bool                                 cs_mask_three_byte_tile3_18;
    bool                                 cs_mask_three_byte_tile3_17;
    bool                                 cs_mask_three_byte_tile3_16;
    bool                                 cs_mask_three_byte_tile3_15;
    bool                                 cs_mask_three_byte_tile3_14;
    bool                                 cs_mask_three_byte_tile3_13;
    bool                                 cs_mask_three_byte_tile3_12;
    bool                                 cs_mask_three_byte_tile3_11;
    bool                                 cs_mask_three_byte_tile3_10;
    bool                                 cs_mask_three_byte_tile3_9;
    bool                                 cs_mask_three_byte_tile3_8;
    bool                                 cs_mask_three_byte_tile3_7;
    bool                                 cs_mask_three_byte_tile3_6;
    bool                                 cs_mask_three_byte_tile3_5;
    bool                                 cs_mask_three_byte_tile3_4;
    bool                                 cs_mask_three_byte_tile3_3;
    bool                                 cs_mask_three_byte_tile3_2;
    bool                                 cs_mask_three_byte_tile3_1;
    bool                                 cs_mask_three_byte_tile3_0;
    bool                                 cs_mask_three_byte_tile2_23;
    bool                                 cs_mask_three_byte_tile2_22;
    bool                                 cs_mask_three_byte_tile2_21;
    bool                                 cs_mask_three_byte_tile2_20;
    bool                                 cs_mask_three_byte_tile2_19;
    bool                                 cs_mask_three_byte_tile2_18;
    bool                                 cs_mask_three_byte_tile2_17;
    bool                                 cs_mask_three_byte_tile2_16;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_2_SIPF2_pack(__attribute__((unused)) void * restrict dst,
                                                       __attribute__((unused)) const struct ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_2_SIPF2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_mask_three_byte_tile3_23, 31, 31) |
      __pvr_uint(values->cs_mask_three_byte_tile3_22, 30, 30) |
      __pvr_uint(values->cs_mask_three_byte_tile3_21, 29, 29) |
      __pvr_uint(values->cs_mask_three_byte_tile3_20, 28, 28) |
      __pvr_uint(values->cs_mask_three_byte_tile3_19, 27, 27) |
      __pvr_uint(values->cs_mask_three_byte_tile3_18, 26, 26) |
      __pvr_uint(values->cs_mask_three_byte_tile3_17, 25, 25) |
      __pvr_uint(values->cs_mask_three_byte_tile3_16, 24, 24) |
      __pvr_uint(values->cs_mask_three_byte_tile3_15, 23, 23) |
      __pvr_uint(values->cs_mask_three_byte_tile3_14, 22, 22) |
      __pvr_uint(values->cs_mask_three_byte_tile3_13, 21, 21) |
      __pvr_uint(values->cs_mask_three_byte_tile3_12, 20, 20) |
      __pvr_uint(values->cs_mask_three_byte_tile3_11, 19, 19) |
      __pvr_uint(values->cs_mask_three_byte_tile3_10, 18, 18) |
      __pvr_uint(values->cs_mask_three_byte_tile3_9, 17, 17) |
      __pvr_uint(values->cs_mask_three_byte_tile3_8, 16, 16) |
      __pvr_uint(values->cs_mask_three_byte_tile3_7, 15, 15) |
      __pvr_uint(values->cs_mask_three_byte_tile3_6, 14, 14) |
      __pvr_uint(values->cs_mask_three_byte_tile3_5, 13, 13) |
      __pvr_uint(values->cs_mask_three_byte_tile3_4, 12, 12) |
      __pvr_uint(values->cs_mask_three_byte_tile3_3, 11, 11) |
      __pvr_uint(values->cs_mask_three_byte_tile3_2, 10, 10) |
      __pvr_uint(values->cs_mask_three_byte_tile3_1, 9, 9) |
      __pvr_uint(values->cs_mask_three_byte_tile3_0, 8, 8) |
      __pvr_uint(values->cs_mask_three_byte_tile2_23, 7, 7) |
      __pvr_uint(values->cs_mask_three_byte_tile2_22, 6, 6) |
      __pvr_uint(values->cs_mask_three_byte_tile2_21, 5, 5) |
      __pvr_uint(values->cs_mask_three_byte_tile2_20, 4, 4) |
      __pvr_uint(values->cs_mask_three_byte_tile2_19, 3, 3) |
      __pvr_uint(values->cs_mask_three_byte_tile2_18, 2, 2) |
      __pvr_uint(values->cs_mask_three_byte_tile2_17, 1, 1) |
      __pvr_uint(values->cs_mask_three_byte_tile2_16, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_2_SIPF2_unpack(__attribute__((unused)) const void * restrict src,
                                                         __attribute__((unused)) struct ROGUE_IPF_BYTE_BASED_MASK_THREE_BYTE_WORD_2_SIPF2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_mask_three_byte_tile3_23 = __pvr_uint_unpack(dw[0], 31, 31);
    values->cs_mask_three_byte_tile3_22 = __pvr_uint_unpack(dw[0], 30, 30);
    values->cs_mask_three_byte_tile3_21 = __pvr_uint_unpack(dw[0], 29, 29);
    values->cs_mask_three_byte_tile3_20 = __pvr_uint_unpack(dw[0], 28, 28);
    values->cs_mask_three_byte_tile3_19 = __pvr_uint_unpack(dw[0], 27, 27);
    values->cs_mask_three_byte_tile3_18 = __pvr_uint_unpack(dw[0], 26, 26);
    values->cs_mask_three_byte_tile3_17 = __pvr_uint_unpack(dw[0], 25, 25);
    values->cs_mask_three_byte_tile3_16 = __pvr_uint_unpack(dw[0], 24, 24);
    values->cs_mask_three_byte_tile3_15 = __pvr_uint_unpack(dw[0], 23, 23);
    values->cs_mask_three_byte_tile3_14 = __pvr_uint_unpack(dw[0], 22, 22);
    values->cs_mask_three_byte_tile3_13 = __pvr_uint_unpack(dw[0], 21, 21);
    values->cs_mask_three_byte_tile3_12 = __pvr_uint_unpack(dw[0], 20, 20);
    values->cs_mask_three_byte_tile3_11 = __pvr_uint_unpack(dw[0], 19, 19);
    values->cs_mask_three_byte_tile3_10 = __pvr_uint_unpack(dw[0], 18, 18);
    values->cs_mask_three_byte_tile3_9 = __pvr_uint_unpack(dw[0], 17, 17);
    values->cs_mask_three_byte_tile3_8 = __pvr_uint_unpack(dw[0], 16, 16);
    values->cs_mask_three_byte_tile3_7 = __pvr_uint_unpack(dw[0], 15, 15);
    values->cs_mask_three_byte_tile3_6 = __pvr_uint_unpack(dw[0], 14, 14);
    values->cs_mask_three_byte_tile3_5 = __pvr_uint_unpack(dw[0], 13, 13);
    values->cs_mask_three_byte_tile3_4 = __pvr_uint_unpack(dw[0], 12, 12);
    values->cs_mask_three_byte_tile3_3 = __pvr_uint_unpack(dw[0], 11, 11);
    values->cs_mask_three_byte_tile3_2 = __pvr_uint_unpack(dw[0], 10, 10);
    values->cs_mask_three_byte_tile3_1 = __pvr_uint_unpack(dw[0], 9, 9);
    values->cs_mask_three_byte_tile3_0 = __pvr_uint_unpack(dw[0], 8, 8);
    values->cs_mask_three_byte_tile2_23 = __pvr_uint_unpack(dw[0], 7, 7);
    values->cs_mask_three_byte_tile2_22 = __pvr_uint_unpack(dw[0], 6, 6);
    values->cs_mask_three_byte_tile2_21 = __pvr_uint_unpack(dw[0], 5, 5);
    values->cs_mask_three_byte_tile2_20 = __pvr_uint_unpack(dw[0], 4, 4);
    values->cs_mask_three_byte_tile2_19 = __pvr_uint_unpack(dw[0], 3, 3);
    values->cs_mask_three_byte_tile2_18 = __pvr_uint_unpack(dw[0], 2, 2);
    values->cs_mask_three_byte_tile2_17 = __pvr_uint_unpack(dw[0], 1, 1);
    values->cs_mask_three_byte_tile2_16 = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_IPF_COMPRESSION_SIZE_WORD_length      1
#define ROGUE_IPF_COMPRESSION_SIZE_WORD_header  \


struct ROGUE_IPF_COMPRESSION_SIZE_WORD {
    uint32_t                             cs_isp_comp_table_size;
    uint32_t                             cs_tsp_comp_format_size;
    uint32_t                             cs_tsp_comp_table_size;
    uint32_t                             cs_tsp_comp_vertex_size;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_COMPRESSION_SIZE_WORD_pack(__attribute__((unused)) void * restrict dst,
                                     __attribute__((unused)) const struct ROGUE_IPF_COMPRESSION_SIZE_WORD * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_isp_comp_table_size, 27, 31) |
      __pvr_uint(values->cs_tsp_comp_format_size, 21, 26) |
      __pvr_uint(values->cs_tsp_comp_table_size, 9, 18) |
      __pvr_uint(values->cs_tsp_comp_vertex_size, 0, 8);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_COMPRESSION_SIZE_WORD_unpack(__attribute__((unused)) const void * restrict src,
                                       __attribute__((unused)) struct ROGUE_IPF_COMPRESSION_SIZE_WORD * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_isp_comp_table_size = __pvr_uint_unpack(dw[0], 27, 31);
    values->cs_tsp_comp_format_size = __pvr_uint_unpack(dw[0], 21, 26);
    values->cs_tsp_comp_table_size = __pvr_uint_unpack(dw[0], 9, 18);
    values->cs_tsp_comp_vertex_size = __pvr_uint_unpack(dw[0], 0, 8);
}

#define ROGUE_IPF_ISP_COMPRESSION_WORD_0_length      1
#define ROGUE_IPF_ISP_COMPRESSION_WORD_0_header \


struct ROGUE_IPF_ISP_COMPRESSION_WORD_0 {
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_z1;
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_z0;
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_y2;
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_y1;
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_y0;
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_x2;
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_x1;
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_x0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_COMPRESSION_WORD_0_pack(__attribute__((unused)) void * restrict dst,
                                      __attribute__((unused)) const struct ROGUE_IPF_ISP_COMPRESSION_WORD_0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cf_isp_comp_fmt_z1, 28, 31) |
      __pvr_uint(values->cf_isp_comp_fmt_z0, 24, 27) |
      __pvr_uint(values->cf_isp_comp_fmt_y2, 20, 23) |
      __pvr_uint(values->cf_isp_comp_fmt_y1, 16, 19) |
      __pvr_uint(values->cf_isp_comp_fmt_y0, 12, 15) |
      __pvr_uint(values->cf_isp_comp_fmt_x2, 8, 11) |
      __pvr_uint(values->cf_isp_comp_fmt_x1, 4, 7) |
      __pvr_uint(values->cf_isp_comp_fmt_x0, 0, 3);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_COMPRESSION_WORD_0_unpack(__attribute__((unused)) const void * restrict src,
                                        __attribute__((unused)) struct ROGUE_IPF_ISP_COMPRESSION_WORD_0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cf_isp_comp_fmt_z1 = __pvr_uint_unpack(dw[0], 28, 31);
    values->cf_isp_comp_fmt_z0 = __pvr_uint_unpack(dw[0], 24, 27);
    values->cf_isp_comp_fmt_y2 = __pvr_uint_unpack(dw[0], 20, 23);
    values->cf_isp_comp_fmt_y1 = __pvr_uint_unpack(dw[0], 16, 19);
    values->cf_isp_comp_fmt_y0 = __pvr_uint_unpack(dw[0], 12, 15);
    values->cf_isp_comp_fmt_x2 = __pvr_uint_unpack(dw[0], 8, 11);
    values->cf_isp_comp_fmt_x1 = __pvr_uint_unpack(dw[0], 4, 7);
    values->cf_isp_comp_fmt_x0 = __pvr_uint_unpack(dw[0], 0, 3);
}

#define ROGUE_IPF_ISP_COMPRESSION_WORD_1_length      1
#define ROGUE_IPF_ISP_COMPRESSION_WORD_1_header \


struct ROGUE_IPF_ISP_COMPRESSION_WORD_1 {
    bool                                 vf_prim_msaa;
    bool                                 vf_prim_id_pres;
    bool                                 vf_vertex_clipped;
    uint32_t                             vf_vertex_total;
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_z3;
    enum ROGUE_IPF_COMPRESSION_FORMAT    cf_isp_comp_fmt_z2;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_COMPRESSION_WORD_1_pack(__attribute__((unused)) void * restrict dst,
                                      __attribute__((unused)) const struct ROGUE_IPF_ISP_COMPRESSION_WORD_1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->vf_prim_msaa, 16, 16) |
      __pvr_uint(values->vf_prim_id_pres, 15, 15) |
      __pvr_uint(values->vf_vertex_clipped, 14, 14) |
      __pvr_uint(values->vf_vertex_total, 8, 13) |
      __pvr_uint(values->cf_isp_comp_fmt_z3, 4, 7) |
      __pvr_uint(values->cf_isp_comp_fmt_z2, 0, 3);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_COMPRESSION_WORD_1_unpack(__attribute__((unused)) const void * restrict src,
                                        __attribute__((unused)) struct ROGUE_IPF_ISP_COMPRESSION_WORD_1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->vf_prim_msaa = __pvr_uint_unpack(dw[0], 16, 16);
    values->vf_prim_id_pres = __pvr_uint_unpack(dw[0], 15, 15);
    values->vf_vertex_clipped = __pvr_uint_unpack(dw[0], 14, 14);
    values->vf_vertex_total = __pvr_uint_unpack(dw[0], 8, 13);
    values->cf_isp_comp_fmt_z3 = __pvr_uint_unpack(dw[0], 4, 7);
    values->cf_isp_comp_fmt_z2 = __pvr_uint_unpack(dw[0], 0, 3);
}

#define ROGUE_IPF_INDEX_DATA_length            1
#define ROGUE_IPF_INDEX_DATA_header             \


struct ROGUE_IPF_INDEX_DATA {
    bool                                 ix_edge_flag1_ab;
    uint32_t                             ix_index1_0;
    bool                                 ix_bf_flag0;
    bool                                 ix_edge_flag0_ca;
    uint32_t                             ix_index0_2;
    bool                                 ix_edge_flag0_bc;
    uint32_t                             ix_index0_1;
    bool                                 ix_edge_flag0_ab;
    uint32_t                             ix_index0_0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_INDEX_DATA_pack(__attribute__((unused)) void * restrict dst,
                          __attribute__((unused)) const struct ROGUE_IPF_INDEX_DATA * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->ix_edge_flag1_ab, 30, 30) |
      __pvr_uint(values->ix_index1_0, 24, 29) |
      __pvr_uint(values->ix_bf_flag0, 23, 23) |
      __pvr_uint(values->ix_edge_flag0_ca, 22, 22) |
      __pvr_uint(values->ix_index0_2, 16, 21) |
      __pvr_uint(values->ix_edge_flag0_bc, 14, 14) |
      __pvr_uint(values->ix_index0_1, 8, 13) |
      __pvr_uint(values->ix_edge_flag0_ab, 6, 6) |
      __pvr_uint(values->ix_index0_0, 0, 5);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_INDEX_DATA_unpack(__attribute__((unused)) const void * restrict src,
                            __attribute__((unused)) struct ROGUE_IPF_INDEX_DATA * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->ix_edge_flag1_ab = __pvr_uint_unpack(dw[0], 30, 30);
    values->ix_index1_0 = __pvr_uint_unpack(dw[0], 24, 29);
    values->ix_bf_flag0 = __pvr_uint_unpack(dw[0], 23, 23);
    values->ix_edge_flag0_ca = __pvr_uint_unpack(dw[0], 22, 22);
    values->ix_index0_2 = __pvr_uint_unpack(dw[0], 16, 21);
    values->ix_edge_flag0_bc = __pvr_uint_unpack(dw[0], 14, 14);
    values->ix_index0_1 = __pvr_uint_unpack(dw[0], 8, 13);
    values->ix_edge_flag0_ab = __pvr_uint_unpack(dw[0], 6, 6);
    values->ix_index0_0 = __pvr_uint_unpack(dw[0], 0, 5);
}

#define ROGUE_IPF_ISP_VERTEX_XY_length         1
#define ROGUE_IPF_ISP_VERTEX_XY_header          \


struct ROGUE_IPF_ISP_VERTEX_XY {
    bool                                 sign;
    uint32_t                             integer;
    uint32_t                             frac;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_XY_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_IPF_ISP_VERTEX_XY * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->sign, 23, 23) |
      __pvr_uint(values->integer, 8, 22) |
      __pvr_uint(values->frac, 0, 7);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_XY_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_IPF_ISP_VERTEX_XY * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->sign = __pvr_uint_unpack(dw[0], 23, 23);
    values->integer = __pvr_uint_unpack(dw[0], 8, 22);
    values->frac = __pvr_uint_unpack(dw[0], 0, 7);
}

#define ROGUE_IPF_ISP_VERTEX_XY_SIPF_length      1
#define ROGUE_IPF_ISP_VERTEX_XY_SIPF_header     \


/* Helper macros for integer */
#define ROGUE_IPF_ISP_VERTEX_XY_SIPF_INTEGER_MAX_VAL 8191

/* Helper macros for frac */
#define ROGUE_IPF_ISP_VERTEX_XY_SIPF_FRAC_MAX_VAL 15

struct ROGUE_IPF_ISP_VERTEX_XY_SIPF {
    uint32_t                             integer;
    uint32_t                             frac;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_XY_SIPF_pack(__attribute__((unused)) void * restrict dst,
                                  __attribute__((unused)) const struct ROGUE_IPF_ISP_VERTEX_XY_SIPF * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->integer, 4, 16) |
      __pvr_uint(values->frac, 0, 3);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_XY_SIPF_unpack(__attribute__((unused)) const void * restrict src,
                                    __attribute__((unused)) struct ROGUE_IPF_ISP_VERTEX_XY_SIPF * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->integer = __pvr_uint_unpack(dw[0], 4, 16);
    values->frac = __pvr_uint_unpack(dw[0], 0, 3);
}

#define ROGUE_IPF_ISP_VERTEX_WORD_SIPF_length      2
#define ROGUE_IPF_ISP_VERTEX_WORD_SIPF_header   \


struct ROGUE_IPF_ISP_VERTEX_WORD_SIPF {
    uint32_t                             z;
    uint32_t                             y;
    uint32_t                             x;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_SIPF_pack(__attribute__((unused)) void * restrict dst,
                                    __attribute__((unused)) const struct ROGUE_IPF_ISP_VERTEX_WORD_SIPF * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->y, 17, 33) |
      __pvr_uint(values->z, 34, 63) |
      __pvr_uint(values->x, 0, 16);
    dw[0] = v0;
    dw[1] = v0 >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_SIPF_unpack(__attribute__((unused)) const void * restrict src,
                                      __attribute__((unused)) struct ROGUE_IPF_ISP_VERTEX_WORD_SIPF * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->y = __pvr_uint_unpack(v0, 17, 33);
    values->z = __pvr_uint_unpack(v0, 34, 63);
    values->x = __pvr_uint_unpack(v0, 0, 16);
}

#define ROGUE_IPF_ISP_VERTEX_WORD_0_length      1
#define ROGUE_IPF_ISP_VERTEX_WORD_0_header      \


struct ROGUE_IPF_ISP_VERTEX_WORD_0 {
    uint32_t                             y0;
    uint32_t                             x0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_0_pack(__attribute__((unused)) void * restrict dst,
                                 __attribute__((unused)) const struct ROGUE_IPF_ISP_VERTEX_WORD_0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->y0, 24, 31) |
      __pvr_uint(values->x0, 0, 23);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_0_unpack(__attribute__((unused)) const void * restrict src,
                                   __attribute__((unused)) struct ROGUE_IPF_ISP_VERTEX_WORD_0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->y0 = __pvr_uint_unpack(dw[0], 24, 31);
    values->x0 = __pvr_uint_unpack(dw[0], 0, 23);
}

#define ROGUE_IPF_ISP_VERTEX_WORD_1_length      1
#define ROGUE_IPF_ISP_VERTEX_WORD_1_header      \


/* Helper macros for y0 */
#define ROGUE_IPF_ISP_VERTEX_WORD_1_Y0_SHIFT     8

struct ROGUE_IPF_ISP_VERTEX_WORD_1 {
    uint32_t                             z0;
    uint32_t                             y0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_1_pack(__attribute__((unused)) void * restrict dst,
                                 __attribute__((unused)) const struct ROGUE_IPF_ISP_VERTEX_WORD_1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->z0, 16, 31) |
      __pvr_uint(values->y0, 0, 15);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_1_unpack(__attribute__((unused)) const void * restrict src,
                                   __attribute__((unused)) struct ROGUE_IPF_ISP_VERTEX_WORD_1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->z0 = __pvr_uint_unpack(dw[0], 16, 31);
    values->y0 = __pvr_uint_unpack(dw[0], 0, 15);
}

#define ROGUE_IPF_ISP_VERTEX_WORD_2_length      1
#define ROGUE_IPF_ISP_VERTEX_WORD_2_header      \


/* Helper macros for z0 */
#define ROGUE_IPF_ISP_VERTEX_WORD_2_Z0_SHIFT     16

struct ROGUE_IPF_ISP_VERTEX_WORD_2 {
    uint32_t                             x1;
    uint32_t                             z0;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_2_pack(__attribute__((unused)) void * restrict dst,
                                 __attribute__((unused)) const struct ROGUE_IPF_ISP_VERTEX_WORD_2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->x1, 16, 31) |
      __pvr_uint(values->z0, 0, 15);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_2_unpack(__attribute__((unused)) const void * restrict src,
                                   __attribute__((unused)) struct ROGUE_IPF_ISP_VERTEX_WORD_2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->x1 = __pvr_uint_unpack(dw[0], 16, 31);
    values->z0 = __pvr_uint_unpack(dw[0], 0, 15);
}

#define ROGUE_IPF_ISP_VERTEX_WORD_3_length      1
#define ROGUE_IPF_ISP_VERTEX_WORD_3_header      \


/* Helper macros for x1 */
#define ROGUE_IPF_ISP_VERTEX_WORD_3_X1_SHIFT     16

struct ROGUE_IPF_ISP_VERTEX_WORD_3 {
    uint32_t                             y1;
    uint32_t                             x1;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_3_pack(__attribute__((unused)) void * restrict dst,
                                 __attribute__((unused)) const struct ROGUE_IPF_ISP_VERTEX_WORD_3 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->y1, 8, 31) |
      __pvr_uint(values->x1, 0, 7);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_3_unpack(__attribute__((unused)) const void * restrict src,
                                   __attribute__((unused)) struct ROGUE_IPF_ISP_VERTEX_WORD_3 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->y1 = __pvr_uint_unpack(dw[0], 8, 31);
    values->x1 = __pvr_uint_unpack(dw[0], 0, 7);
}

#define ROGUE_IPF_ISP_VERTEX_WORD_4_length      1
#define ROGUE_IPF_ISP_VERTEX_WORD_4_header      \


struct ROGUE_IPF_ISP_VERTEX_WORD_4 {
    uint32_t                             z1;
};

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_4_pack(__attribute__((unused)) void * restrict dst,
                                 __attribute__((unused)) const struct ROGUE_IPF_ISP_VERTEX_WORD_4 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->z1, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_IPF_ISP_VERTEX_WORD_4_unpack(__attribute__((unused)) const void * restrict src,
                                   __attribute__((unused)) struct ROGUE_IPF_ISP_VERTEX_WORD_4 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->z1 = __pvr_uint_unpack(dw[0], 0, 31);
}

#endif /* IPF_H */
