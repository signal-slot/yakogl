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

#ifndef PDS_H
#define PDS_H

#include "csbgen/pvr_packet_helpers.h"

#ifndef __OPENCL_VERSION__
#define __constant
#endif



enum ROGUE_PDSINST_CMODE_LD {
    ROGUE_PDSINST_CMODE_LD_CACHED        =      0,
    ROGUE_PDSINST_CMODE_LD_BYPASS        =      1,
    ROGUE_PDSINST_CMODE_LD_FORCE_LINE_FILL =      2,
};

static __constant const char *
ROGUE_PDSINST_CMODE_LD_to_str(const enum ROGUE_PDSINST_CMODE_LD value)
{
    switch (value) {
    case ROGUE_PDSINST_CMODE_LD_CACHED: return "CACHED";
    case ROGUE_PDSINST_CMODE_LD_BYPASS: return "BYPASS";
    case ROGUE_PDSINST_CMODE_LD_FORCE_LINE_FILL: return "FORCE_LINE_FILL";
    default: return NULL;
    }
}

enum ROGUE_PDSINST_DOUTD_DEST {
    ROGUE_PDSINST_DOUTD_DEST_UNIFIED_STORE =      0,
    ROGUE_PDSINST_DOUTD_DEST_COMMON_STORE =      1,
};

static __constant const char *
ROGUE_PDSINST_DOUTD_DEST_to_str(const enum ROGUE_PDSINST_DOUTD_DEST value)
{
    switch (value) {
    case ROGUE_PDSINST_DOUTD_DEST_UNIFIED_STORE: return "UNIFIED_STORE";
    case ROGUE_PDSINST_DOUTD_DEST_COMMON_STORE: return "COMMON_STORE";
    default: return NULL;
    }
}

enum ROGUE_PDSINST_DEST {
    ROGUE_PDSINST_DEST_UNIFIED_STORE     =      0,
    ROGUE_PDSINST_DEST_COMMON_STORE      =      1,
};

static __constant const char *
ROGUE_PDSINST_DEST_to_str(const enum ROGUE_PDSINST_DEST value)
{
    switch (value) {
    case ROGUE_PDSINST_DEST_UNIFIED_STORE: return "UNIFIED_STORE";
    case ROGUE_PDSINST_DEST_COMMON_STORE: return "COMMON_STORE";
    default: return NULL;
    }
}

enum ROGUE_PDSINST_DOUTI_SHADEMODEL {
    ROGUE_PDSINST_DOUTI_SHADEMODEL_FLAT_VERTEX0 =      0,
    ROGUE_PDSINST_DOUTI_SHADEMODEL_FLAT_VERTEX1 =      1,
    ROGUE_PDSINST_DOUTI_SHADEMODEL_FLAT_VERTEX2 =      2,
    ROGUE_PDSINST_DOUTI_SHADEMODEL_GOURUAD =      3,
};

static __constant const char *
ROGUE_PDSINST_DOUTI_SHADEMODEL_to_str(const enum ROGUE_PDSINST_DOUTI_SHADEMODEL value)
{
    switch (value) {
    case ROGUE_PDSINST_DOUTI_SHADEMODEL_FLAT_VERTEX0: return "FLAT_VERTEX0";
    case ROGUE_PDSINST_DOUTI_SHADEMODEL_FLAT_VERTEX1: return "FLAT_VERTEX1";
    case ROGUE_PDSINST_DOUTI_SHADEMODEL_FLAT_VERTEX2: return "FLAT_VERTEX2";
    case ROGUE_PDSINST_DOUTI_SHADEMODEL_GOURUAD: return "GOURUAD";
    default: return NULL;
    }
}

enum ROGUE_PDSINST_DOUTI_SIZE {
    ROGUE_PDSINST_DOUTI_SIZE_1D          =      0,
    ROGUE_PDSINST_DOUTI_SIZE_2D          =      1,
    ROGUE_PDSINST_DOUTI_SIZE_3D          =      2,
    ROGUE_PDSINST_DOUTI_SIZE_4D          =      3,
};

static __constant const char *
ROGUE_PDSINST_DOUTI_SIZE_to_str(const enum ROGUE_PDSINST_DOUTI_SIZE value)
{
    switch (value) {
    case ROGUE_PDSINST_DOUTI_SIZE_1D: return "_1D";
    case ROGUE_PDSINST_DOUTI_SIZE_2D: return "_2D";
    case ROGUE_PDSINST_DOUTI_SIZE_3D: return "_3D";
    case ROGUE_PDSINST_DOUTI_SIZE_4D: return "_4D";
    default: return NULL;
    }
}

enum ROGUE_PDSINST_DOUTU_SAMPLE_RATE {
    ROGUE_PDSINST_DOUTU_SAMPLE_RATE_INSTANCE =      0,
    ROGUE_PDSINST_DOUTU_SAMPLE_RATE_SELECTIVE =      1,
    ROGUE_PDSINST_DOUTU_SAMPLE_RATE_FULL =      2,
};

static __constant const char *
ROGUE_PDSINST_DOUTU_SAMPLE_RATE_to_str(const enum ROGUE_PDSINST_DOUTU_SAMPLE_RATE value)
{
    switch (value) {
    case ROGUE_PDSINST_DOUTU_SAMPLE_RATE_INSTANCE: return "INSTANCE";
    case ROGUE_PDSINST_DOUTU_SAMPLE_RATE_SELECTIVE: return "SELECTIVE";
    case ROGUE_PDSINST_DOUTU_SAMPLE_RATE_FULL: return "FULL";
    default: return NULL;
    }
}

enum ROGUE_PDSINST_SLC_MODE_LD {
    ROGUE_PDSINST_SLC_MODE_LD_BYPASS     =      0,
    ROGUE_PDSINST_SLC_MODE_LD_CACHED     =      1,
    ROGUE_PDSINST_SLC_MODE_LD_CACHED_RD_NA =      3,
};

static __constant const char *
ROGUE_PDSINST_SLC_MODE_LD_to_str(const enum ROGUE_PDSINST_SLC_MODE_LD value)
{
    switch (value) {
    case ROGUE_PDSINST_SLC_MODE_LD_BYPASS: return "BYPASS";
    case ROGUE_PDSINST_SLC_MODE_LD_CACHED: return "CACHED";
    case ROGUE_PDSINST_SLC_MODE_LD_CACHED_RD_NA: return "CACHED_RD_NA";
    default: return NULL;
    }
}

enum ROGUE_PDSINST_WORDSIZE {
    ROGUE_PDSINST_WORDSIZE_ONE           =      0,
    ROGUE_PDSINST_WORDSIZE_TWO           =      1,
    ROGUE_PDSINST_WORDSIZE_THREE         =      2,
    ROGUE_PDSINST_WORDSIZE_FOUR          =      3,
};

static __constant const char *
ROGUE_PDSINST_WORDSIZE_to_str(const enum ROGUE_PDSINST_WORDSIZE value)
{
    switch (value) {
    case ROGUE_PDSINST_WORDSIZE_ONE: return "ONE";
    case ROGUE_PDSINST_WORDSIZE_TWO: return "TWO";
    case ROGUE_PDSINST_WORDSIZE_THREE: return "THREE";
    case ROGUE_PDSINST_WORDSIZE_FOUR: return "FOUR";
    default: return NULL;
    }
}

#define ROGUE_PDSINST_DDMAD_FIELDS_SRC3_length      2
#define ROGUE_PDSINST_DDMAD_FIELDS_SRC3_header  \


struct ROGUE_PDSINST_DDMAD_FIELDS_SRC3 {
/* if PDS_DDMADT is supported use: */
    uint32_t                             msize;
    bool                                 test;
/* endif PDS_DDMADT */
    bool                                 last;
    enum ROGUE_PDSINST_WORDSIZE          wordsize;
    enum ROGUE_PDSINST_DEST              dest;
    enum ROGUE_PDSINST_CMODE_LD          cmode;
    uint32_t                             a0;
    bool                                 repeat;
    uint32_t                             bsize;
};

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DDMAD_FIELDS_SRC3_pack(__attribute__((unused)) void * restrict dst,
                                     __attribute__((unused)) const struct ROGUE_PDSINST_DDMAD_FIELDS_SRC3 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->last, 31, 31) |
      __pvr_uint(values->wordsize, 29, 30) |
      __pvr_uint(values->dest, 28, 28) |
      __pvr_uint(values->cmode, 26, 27) |
      __pvr_uint(values->a0, 13, 25) |
      __pvr_uint(values->repeat, 12, 12) |
      __pvr_uint(values->bsize, 0, 11);

    dw[1] =
      __pvr_uint(values->msize, 1, 31) |
      __pvr_uint(values->test, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DDMAD_FIELDS_SRC3_unpack(__attribute__((unused)) const void * restrict src,
                                       __attribute__((unused)) struct ROGUE_PDSINST_DDMAD_FIELDS_SRC3 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->last = __pvr_uint_unpack(dw[0], 31, 31);
    values->wordsize = __pvr_uint_unpack(dw[0], 29, 30);
    values->dest = __pvr_uint_unpack(dw[0], 28, 28);
    values->cmode = __pvr_uint_unpack(dw[0], 26, 27);
    values->a0 = __pvr_uint_unpack(dw[0], 13, 25);
    values->repeat = __pvr_uint_unpack(dw[0], 12, 12);
    values->bsize = __pvr_uint_unpack(dw[0], 0, 11);
    values->msize = __pvr_uint_unpack(dw[1], 1, 31);
    values->test = __pvr_uint_unpack(dw[1], 0, 0);
}

#define ROGUE_PDSINST_DOUTU_SRC0_length        2
#define ROGUE_PDSINST_DOUTU_SRC0_header         \


/* Helper macros for temps */
#define ROGUE_PDSINST_DOUTU_SRC0_TEMPS_UNIT_SIZE 2

/* Helper macros for exe_off */
#define ROGUE_PDSINST_DOUTU_SRC0_EXE_OFF_ALIGNMENT 4

struct ROGUE_PDSINST_DOUTU_SRC0 {
    bool                                 dual_phase;
    uint32_t                             temps;
    enum ROGUE_PDSINST_DOUTU_SAMPLE_RATE sample_rate;
    __pvr_address_type                   exe_off;
};

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DOUTU_SRC0_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_PDSINST_DOUTU_SRC0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->exe_off, 2, 2, 31) | 0;

    dw[1] =
      __pvr_uint(values->dual_phase, 9, 9) |
      __pvr_uint(values->temps, 3, 8) |
      __pvr_uint(values->sample_rate, 1, 2);
}

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DOUTU_SRC0_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_PDSINST_DOUTU_SRC0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->exe_off = __pvr_address_unpack(dw[0], 2, 2, 31);
    values->dual_phase = __pvr_uint_unpack(dw[1], 9, 9);
    values->temps = __pvr_uint_unpack(dw[1], 3, 8);
    values->sample_rate = __pvr_uint_unpack(dw[1], 1, 2);
}

#define ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC0_length      2
#define ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC0_header\


/* Helper macros for sbase */
#define ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC0_SBASE_ALIGNMENT 1

struct ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC0 {
/* if SLC_MCU_CACHE_CONTROLS is supported use: */
    enum ROGUE_PDSINST_SLC_MODE_LD       slcmode;
/* endif SLC_MCU_CACHE_CONTROLS */
    uint32_t                             doffset;
    __pvr_address_type                   sbase;
};

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC0_pack(__attribute__((unused)) void * restrict dst,
                                          __attribute__((unused)) const struct ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->slcmode, 60, 61) |
      __pvr_uint(values->doffset, 40, 52);
    const uint64_t v0_address =
      __pvr_address(values->sbase, 0, 0, 39);
    dw[0] = v0_address | v0;
    dw[1] = ((v0_address) >> 32) | (v0 >> 32);
}

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC0_unpack(__attribute__((unused)) const void * restrict src,
                                            __attribute__((unused)) struct ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->sbase = __pvr_address_unpack(v0, 0, 0, 39);
    values->slcmode = __pvr_uint_unpack(v0, 60, 61);
    values->doffset = __pvr_uint_unpack(v0, 40, 52);
}

#define ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC1_length      1
#define ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC1_header\


/* Helper macros for a0 */
#define ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC1_A0_UNIT_SIZE 4

struct ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC1 {
    bool                                 last;
    enum ROGUE_PDSINST_WORDSIZE          wordsize;
    enum ROGUE_PDSINST_DOUTD_DEST        dest;
    enum ROGUE_PDSINST_CMODE_LD          cmode;
    uint32_t                             a0;
    bool                                 repeat;
    uint32_t                             bsize;
};

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC1_pack(__attribute__((unused)) void * restrict dst,
                                          __attribute__((unused)) const struct ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->last, 31, 31) |
      __pvr_uint(values->wordsize, 29, 30) |
      __pvr_uint(values->dest, 28, 28) |
      __pvr_uint(values->cmode, 26, 27) |
      __pvr_uint(values->a0, 13, 25) |
      __pvr_uint(values->repeat, 12, 12) |
      __pvr_uint(values->bsize, 0, 11);
}

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC1_unpack(__attribute__((unused)) const void * restrict src,
                                            __attribute__((unused)) struct ROGUE_PDSINST_DOUT_FIELDS_DOUTD_SRC1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->last = __pvr_uint_unpack(dw[0], 31, 31);
    values->wordsize = __pvr_uint_unpack(dw[0], 29, 30);
    values->dest = __pvr_uint_unpack(dw[0], 28, 28);
    values->cmode = __pvr_uint_unpack(dw[0], 26, 27);
    values->a0 = __pvr_uint_unpack(dw[0], 13, 25);
    values->repeat = __pvr_uint_unpack(dw[0], 12, 12);
    values->bsize = __pvr_uint_unpack(dw[0], 0, 11);
}

#define ROGUE_PDSINST_DOUT_FIELDS_DOUTI_SRC_length      1
#define ROGUE_PDSINST_DOUT_FIELDS_DOUTI_SRC_header\


struct ROGUE_PDSINST_DOUT_FIELDS_DOUTI_SRC {
    bool                                 depthbias;
    bool                                 primitiveid;
    enum ROGUE_PDSINST_DOUTI_SHADEMODEL  shademodel;
    bool                                 pointsprite;
    bool                                 wraps;
    bool                                 wrapv;
    bool                                 wrapu;
    enum ROGUE_PDSINST_DOUTI_SIZE        size;
    bool                                 f16;
    bool                                 perspective;
    uint32_t                             f32_offset;
    uint32_t                             f16_offset;
};

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DOUT_FIELDS_DOUTI_SRC_pack(__attribute__((unused)) void * restrict dst,
                                         __attribute__((unused)) const struct ROGUE_PDSINST_DOUT_FIELDS_DOUTI_SRC * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->depthbias, 27, 27) |
      __pvr_uint(values->primitiveid, 26, 26) |
      __pvr_uint(values->shademodel, 24, 25) |
      __pvr_uint(values->pointsprite, 23, 23) |
      __pvr_uint(values->wraps, 22, 22) |
      __pvr_uint(values->wrapv, 21, 21) |
      __pvr_uint(values->wrapu, 20, 20) |
      __pvr_uint(values->size, 18, 19) |
      __pvr_uint(values->f16, 17, 17) |
      __pvr_uint(values->perspective, 16, 16) |
      __pvr_uint(values->f32_offset, 8, 15) |
      __pvr_uint(values->f16_offset, 0, 7);
}

static inline __attribute__((always_inline)) void
ROGUE_PDSINST_DOUT_FIELDS_DOUTI_SRC_unpack(__attribute__((unused)) const void * restrict src,
                                           __attribute__((unused)) struct ROGUE_PDSINST_DOUT_FIELDS_DOUTI_SRC * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->depthbias = __pvr_uint_unpack(dw[0], 27, 27);
    values->primitiveid = __pvr_uint_unpack(dw[0], 26, 26);
    values->shademodel = __pvr_uint_unpack(dw[0], 24, 25);
    values->pointsprite = __pvr_uint_unpack(dw[0], 23, 23);
    values->wraps = __pvr_uint_unpack(dw[0], 22, 22);
    values->wrapv = __pvr_uint_unpack(dw[0], 21, 21);
    values->wrapu = __pvr_uint_unpack(dw[0], 20, 20);
    values->size = __pvr_uint_unpack(dw[0], 18, 19);
    values->f16 = __pvr_uint_unpack(dw[0], 17, 17);
    values->perspective = __pvr_uint_unpack(dw[0], 16, 16);
    values->f32_offset = __pvr_uint_unpack(dw[0], 8, 15);
    values->f16_offset = __pvr_uint_unpack(dw[0], 0, 7);
}

#endif /* PDS_H */
