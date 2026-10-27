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

#ifndef CR_H
#define CR_H

#include "csbgen/pvr_packet_helpers.h"

#ifndef __OPENCL_VERSION__
#define __constant
#endif


#define ROGUE_CR_PM_VHEAP_TABLE_SIZE             384

enum ROGUE_CR_COMP_IADDR_TYPE {
    ROGUE_CR_COMP_IADDR_TYPE_INDIRECT_1TILE =      0,
    ROGUE_CR_COMP_IADDR_TYPE_INDIRECT_4TILE =      1,
};

static __constant const char *
ROGUE_CR_COMP_IADDR_TYPE_to_str(const enum ROGUE_CR_COMP_IADDR_TYPE value)
{
    switch (value) {
    case ROGUE_CR_COMP_IADDR_TYPE_INDIRECT_1TILE: return "INDIRECT_1TILE";
    case ROGUE_CR_COMP_IADDR_TYPE_INDIRECT_4TILE: return "INDIRECT_4TILE";
    default: return NULL;
    }
}

enum ROGUE_CR_COMPRESS_SIZE {
    ROGUE_CR_COMPRESS_SIZE_BLOCK_8X8     =      0,
    ROGUE_CR_COMPRESS_SIZE_BLOCK_16X4    =      1,
};

static __constant const char *
ROGUE_CR_COMPRESS_SIZE_to_str(const enum ROGUE_CR_COMPRESS_SIZE value)
{
    switch (value) {
    case ROGUE_CR_COMPRESS_SIZE_BLOCK_8X8: return "BLOCK_8X8";
    case ROGUE_CR_COMPRESS_SIZE_BLOCK_16X4: return "BLOCK_16X4";
    default: return NULL;
    }
}

enum ROGUE_CR_DIR_TYPE {
    ROGUE_CR_DIR_TYPE_TL2BR              =      0,
    ROGUE_CR_DIR_TYPE_TR2BL              =      1,
    ROGUE_CR_DIR_TYPE_BL2TR              =      2,
    ROGUE_CR_DIR_TYPE_BR2TL              =      3,
};

static __constant const char *
ROGUE_CR_DIR_TYPE_to_str(const enum ROGUE_CR_DIR_TYPE value)
{
    switch (value) {
    case ROGUE_CR_DIR_TYPE_TL2BR: return "TL2BR";
    case ROGUE_CR_DIR_TYPE_TR2BL: return "TR2BL";
    case ROGUE_CR_DIR_TYPE_BL2TR: return "BL2TR";
    case ROGUE_CR_DIR_TYPE_BR2TL: return "BR2TL";
    default: return NULL;
    }
}

enum ROGUE_CR_ISP_AA_MODE_TYPE {
    ROGUE_CR_ISP_AA_MODE_TYPE_AA_NONE    =      0,
    ROGUE_CR_ISP_AA_MODE_TYPE_AA_2X      =      1,
    ROGUE_CR_ISP_AA_MODE_TYPE_AA_4X      =      2,
    ROGUE_CR_ISP_AA_MODE_TYPE_AA_8X      =      3,
};

static __constant const char *
ROGUE_CR_ISP_AA_MODE_TYPE_to_str(const enum ROGUE_CR_ISP_AA_MODE_TYPE value)
{
    switch (value) {
    case ROGUE_CR_ISP_AA_MODE_TYPE_AA_NONE: return "AA_NONE";
    case ROGUE_CR_ISP_AA_MODE_TYPE_AA_2X: return "AA_2X";
    case ROGUE_CR_ISP_AA_MODE_TYPE_AA_4X: return "AA_4X";
    case ROGUE_CR_ISP_AA_MODE_TYPE_AA_8X: return "AA_8X";
    default: return NULL;
    }
}

enum ROGUE_CR_ISP_RENDER_MODE_TYPE {
    ROGUE_CR_ISP_RENDER_MODE_TYPE_NORM   =      0,
    ROGUE_CR_ISP_RENDER_MODE_TYPE_FAST_2D =      2,
    ROGUE_CR_ISP_RENDER_MODE_TYPE_FAST_SCALE =      3,
};

static __constant const char *
ROGUE_CR_ISP_RENDER_MODE_TYPE_to_str(const enum ROGUE_CR_ISP_RENDER_MODE_TYPE value)
{
    switch (value) {
    case ROGUE_CR_ISP_RENDER_MODE_TYPE_NORM: return "NORM";
    case ROGUE_CR_ISP_RENDER_MODE_TYPE_FAST_2D: return "FAST_2D";
    case ROGUE_CR_ISP_RENDER_MODE_TYPE_FAST_SCALE: return "FAST_SCALE";
    default: return NULL;
    }
}

enum ROGUE_CR_MEMLAYOUT {
    ROGUE_CR_MEMLAYOUT_LINEAR            =      0,
    ROGUE_CR_MEMLAYOUT_TWIDDLE_2D        =      1,
    ROGUE_CR_MEMLAYOUT_TWIDDLE_3D        =      2,
    ROGUE_CR_MEMLAYOUT_TILED             =      3,
};

static __constant const char *
ROGUE_CR_MEMLAYOUT_to_str(const enum ROGUE_CR_MEMLAYOUT value)
{
    switch (value) {
    case ROGUE_CR_MEMLAYOUT_LINEAR: return "LINEAR";
    case ROGUE_CR_MEMLAYOUT_TWIDDLE_2D: return "TWIDDLE_2D";
    case ROGUE_CR_MEMLAYOUT_TWIDDLE_3D: return "TWIDDLE_3D";
    case ROGUE_CR_MEMLAYOUT_TILED: return "TILED";
    default: return NULL;
    }
}

enum ROGUE_CR_MODE_TYPE {
    ROGUE_CR_MODE_TYPE_DX9               =      0,
    ROGUE_CR_MODE_TYPE_DX10              =      1,
    ROGUE_CR_MODE_TYPE_OGL               =      2,
};

static __constant const char *
ROGUE_CR_MODE_TYPE_to_str(const enum ROGUE_CR_MODE_TYPE value)
{
    switch (value) {
    case ROGUE_CR_MODE_TYPE_DX9: return "DX9";
    case ROGUE_CR_MODE_TYPE_DX10: return "DX10";
    case ROGUE_CR_MODE_TYPE_OGL: return "OGL";
    default: return NULL;
    }
}

enum ROGUE_CR_PIPE_NUM {
    ROGUE_CR_PIPE_NUM_PIPE_ONE           =      0,
    ROGUE_CR_PIPE_NUM_PIPE_TWO           =      1,
    ROGUE_CR_PIPE_NUM_PIPE_THREE         =      2,
    ROGUE_CR_PIPE_NUM_PIPE_FOUR          =      3,
    ROGUE_CR_PIPE_NUM_PIPE_FIVE          =      4,
    ROGUE_CR_PIPE_NUM_PIPE_SIX           =      5,
    ROGUE_CR_PIPE_NUM_PIPE_SEVEN         =      6,
    ROGUE_CR_PIPE_NUM_PIPE_EIGHT         =      7,
    ROGUE_CR_PIPE_NUM_PIPE_NINE          =      8,
    ROGUE_CR_PIPE_NUM_PIPE_TEN           =      9,
    ROGUE_CR_PIPE_NUM_PIPE_ELEVEN        =     10,
    ROGUE_CR_PIPE_NUM_PIPE_TWELVE        =     11,
    ROGUE_CR_PIPE_NUM_PIPE_THIRTEEN      =     12,
    ROGUE_CR_PIPE_NUM_PIPE_FOURTEEN      =     13,
    ROGUE_CR_PIPE_NUM_PIPE_FIFTEEN       =     14,
    ROGUE_CR_PIPE_NUM_PIPE_SIXTEEN       =     15,
};

static __constant const char *
ROGUE_CR_PIPE_NUM_to_str(const enum ROGUE_CR_PIPE_NUM value)
{
    switch (value) {
    case ROGUE_CR_PIPE_NUM_PIPE_ONE: return "PIPE_ONE";
    case ROGUE_CR_PIPE_NUM_PIPE_TWO: return "PIPE_TWO";
    case ROGUE_CR_PIPE_NUM_PIPE_THREE: return "PIPE_THREE";
    case ROGUE_CR_PIPE_NUM_PIPE_FOUR: return "PIPE_FOUR";
    case ROGUE_CR_PIPE_NUM_PIPE_FIVE: return "PIPE_FIVE";
    case ROGUE_CR_PIPE_NUM_PIPE_SIX: return "PIPE_SIX";
    case ROGUE_CR_PIPE_NUM_PIPE_SEVEN: return "PIPE_SEVEN";
    case ROGUE_CR_PIPE_NUM_PIPE_EIGHT: return "PIPE_EIGHT";
    case ROGUE_CR_PIPE_NUM_PIPE_NINE: return "PIPE_NINE";
    case ROGUE_CR_PIPE_NUM_PIPE_TEN: return "PIPE_TEN";
    case ROGUE_CR_PIPE_NUM_PIPE_ELEVEN: return "PIPE_ELEVEN";
    case ROGUE_CR_PIPE_NUM_PIPE_TWELVE: return "PIPE_TWELVE";
    case ROGUE_CR_PIPE_NUM_PIPE_THIRTEEN: return "PIPE_THIRTEEN";
    case ROGUE_CR_PIPE_NUM_PIPE_FOURTEEN: return "PIPE_FOURTEEN";
    case ROGUE_CR_PIPE_NUM_PIPE_FIFTEEN: return "PIPE_FIFTEEN";
    case ROGUE_CR_PIPE_NUM_PIPE_SIXTEEN: return "PIPE_SIXTEEN";
    default: return NULL;
    }
}

enum ROGUE_CR_PIXEL_WIDTH {
    ROGUE_CR_PIXEL_WIDTH_2REGISTERS      =      0,
    ROGUE_CR_PIXEL_WIDTH_4REGISTERS      =      1,
    ROGUE_CR_PIXEL_WIDTH_8REGISTERS      =      2,
    ROGUE_CR_PIXEL_WIDTH_1REGISTER       =      3,
};

static __constant const char *
ROGUE_CR_PIXEL_WIDTH_to_str(const enum ROGUE_CR_PIXEL_WIDTH value)
{
    switch (value) {
    case ROGUE_CR_PIXEL_WIDTH_2REGISTERS: return "_2REGISTERS";
    case ROGUE_CR_PIXEL_WIDTH_4REGISTERS: return "_4REGISTERS";
    case ROGUE_CR_PIXEL_WIDTH_8REGISTERS: return "_8REGISTERS";
    case ROGUE_CR_PIXEL_WIDTH_1REGISTER: return "_1REGISTER";
    default: return NULL;
    }
}

enum ROGUE_CR_ROTATION_TYPE {
    ROGUE_CR_ROTATION_TYPE_0_DEG         =      0,
    ROGUE_CR_ROTATION_TYPE_90_DEG        =      1,
    ROGUE_CR_ROTATION_TYPE_180_DEG       =      2,
    ROGUE_CR_ROTATION_TYPE_270_DEG       =      3,
};

static __constant const char *
ROGUE_CR_ROTATION_TYPE_to_str(const enum ROGUE_CR_ROTATION_TYPE value)
{
    switch (value) {
    case ROGUE_CR_ROTATION_TYPE_0_DEG: return "_0_DEG";
    case ROGUE_CR_ROTATION_TYPE_90_DEG: return "_90_DEG";
    case ROGUE_CR_ROTATION_TYPE_180_DEG: return "_180_DEG";
    case ROGUE_CR_ROTATION_TYPE_270_DEG: return "_270_DEG";
    default: return NULL;
    }
}

enum ROGUE_CR_SIZE {
    ROGUE_CR_SIZE_1_PIXEL                =      0,
    ROGUE_CR_SIZE_2_PIXEL                =      1,
    ROGUE_CR_SIZE_4_PIXEL                =      2,
    ROGUE_CR_SIZE_8_PIXEL                =      3,
    ROGUE_CR_SIZE_16_PIXEL               =      4,
    ROGUE_CR_SIZE_32_PIXEL               =      5,
    ROGUE_CR_SIZE_64_PIXEL               =      6,
    ROGUE_CR_SIZE_128_PIXEL              =      7,
    ROGUE_CR_SIZE_256_PIXEL              =      8,
    ROGUE_CR_SIZE_512_PIXEL              =      9,
    ROGUE_CR_SIZE_1K_PIXEL               =     10,
    ROGUE_CR_SIZE_2K_PIXEL               =     11,
    ROGUE_CR_SIZE_4K_PIXEL               =     12,
    ROGUE_CR_SIZE_8K_PIXEL               =     13,
    ROGUE_CR_SIZE_16K_PIXEL              =     14,
};

static __constant const char *
ROGUE_CR_SIZE_to_str(const enum ROGUE_CR_SIZE value)
{
    switch (value) {
    case ROGUE_CR_SIZE_1_PIXEL: return "_1_PIXEL";
    case ROGUE_CR_SIZE_2_PIXEL: return "_2_PIXEL";
    case ROGUE_CR_SIZE_4_PIXEL: return "_4_PIXEL";
    case ROGUE_CR_SIZE_8_PIXEL: return "_8_PIXEL";
    case ROGUE_CR_SIZE_16_PIXEL: return "_16_PIXEL";
    case ROGUE_CR_SIZE_32_PIXEL: return "_32_PIXEL";
    case ROGUE_CR_SIZE_64_PIXEL: return "_64_PIXEL";
    case ROGUE_CR_SIZE_128_PIXEL: return "_128_PIXEL";
    case ROGUE_CR_SIZE_256_PIXEL: return "_256_PIXEL";
    case ROGUE_CR_SIZE_512_PIXEL: return "_512_PIXEL";
    case ROGUE_CR_SIZE_1K_PIXEL: return "_1K_PIXEL";
    case ROGUE_CR_SIZE_2K_PIXEL: return "_2K_PIXEL";
    case ROGUE_CR_SIZE_4K_PIXEL: return "_4K_PIXEL";
    case ROGUE_CR_SIZE_8K_PIXEL: return "_8K_PIXEL";
    case ROGUE_CR_SIZE_16K_PIXEL: return "_16K_PIXEL";
    default: return NULL;
    }
}

enum ROGUE_CR_SWIZ {
    ROGUE_CR_SWIZ_SOURCE_CHAN0           =      0,
    ROGUE_CR_SWIZ_SOURCE_CHAN1           =      1,
    ROGUE_CR_SWIZ_SOURCE_CHAN2           =      2,
    ROGUE_CR_SWIZ_SOURCE_CHAN3           =      3,
    ROGUE_CR_SWIZ_ONE                    =      4,
    ROGUE_CR_SWIZ_ZERO                   =      5,
};

static __constant const char *
ROGUE_CR_SWIZ_to_str(const enum ROGUE_CR_SWIZ value)
{
    switch (value) {
    case ROGUE_CR_SWIZ_SOURCE_CHAN0: return "SOURCE_CHAN0";
    case ROGUE_CR_SWIZ_SOURCE_CHAN1: return "SOURCE_CHAN1";
    case ROGUE_CR_SWIZ_SOURCE_CHAN2: return "SOURCE_CHAN2";
    case ROGUE_CR_SWIZ_SOURCE_CHAN3: return "SOURCE_CHAN3";
    case ROGUE_CR_SWIZ_ONE: return "ONE";
    case ROGUE_CR_SWIZ_ZERO: return "ZERO";
    default: return NULL;
    }
}

enum ROGUE_CR_TFBC_LOSSY {
    ROGUE_CR_TFBC_LOSSY_LOSSLESS         =      0,
    ROGUE_CR_TFBC_LOSSY_LOSSY_75         =      1,
    ROGUE_CR_TFBC_LOSSY_LOSSY_50         =      2,
    ROGUE_CR_TFBC_LOSSY_LOSSY_25         =      3,
};

static __constant const char *
ROGUE_CR_TFBC_LOSSY_to_str(const enum ROGUE_CR_TFBC_LOSSY value)
{
    switch (value) {
    case ROGUE_CR_TFBC_LOSSY_LOSSLESS: return "LOSSLESS";
    case ROGUE_CR_TFBC_LOSSY_LOSSY_75: return "LOSSY_75";
    case ROGUE_CR_TFBC_LOSSY_LOSSY_50: return "LOSSY_50";
    case ROGUE_CR_TFBC_LOSSY_LOSSY_25: return "LOSSY_25";
    default: return NULL;
    }
}

enum ROGUE_CR_TWOCOMP_GAMMA {
    ROGUE_CR_TWOCOMP_GAMMA_GAMMA_BOTTOM_CHANNEL =      0,
    ROGUE_CR_TWOCOMP_GAMMA_GAMMA_BOTH_CHANNELS =      1,
};

static __constant const char *
ROGUE_CR_TWOCOMP_GAMMA_to_str(const enum ROGUE_CR_TWOCOMP_GAMMA value)
{
    switch (value) {
    case ROGUE_CR_TWOCOMP_GAMMA_GAMMA_BOTTOM_CHANNEL: return "GAMMA_BOTTOM_CHANNEL";
    case ROGUE_CR_TWOCOMP_GAMMA_GAMMA_BOTH_CHANNELS: return "GAMMA_BOTH_CHANNELS";
    default: return NULL;
    }
}

enum ROGUE_CR_ZLS_FORMAT_TYPE {
    ROGUE_CR_ZLS_FORMAT_TYPE_F32Z        =      0,
    ROGUE_CR_ZLS_FORMAT_TYPE_24BITINT    =      1,
    ROGUE_CR_ZLS_FORMAT_TYPE_16BITINT    =      2,
    ROGUE_CR_ZLS_FORMAT_TYPE_F64Z        =      3,
};

static __constant const char *
ROGUE_CR_ZLS_FORMAT_TYPE_to_str(const enum ROGUE_CR_ZLS_FORMAT_TYPE value)
{
    switch (value) {
    case ROGUE_CR_ZLS_FORMAT_TYPE_F32Z: return "F32Z";
    case ROGUE_CR_ZLS_FORMAT_TYPE_24BITINT: return "_24BITINT";
    case ROGUE_CR_ZLS_FORMAT_TYPE_16BITINT: return "_16BITINT";
    case ROGUE_CR_ZLS_FORMAT_TYPE_F64Z: return "F64Z";
    default: return NULL;
    }
}

#define ROGUE_CR_PM_MTILE_ARRAY_length         2
#define ROGUE_CR_PM_MTILE_ARRAY_header          \


/* Helper macros for base_addr */
#define ROGUE_CR_PM_MTILE_ARRAY_BASE_ADDR_ALIGNMENT 16

struct ROGUE_CR_PM_MTILE_ARRAY {
    __pvr_address_type                   base_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PM_MTILE_ARRAY_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_PM_MTILE_ARRAY * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->base_addr, 4, 4, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PM_MTILE_ARRAY_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_PM_MTILE_ARRAY * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->base_addr = __pvr_address_unpack(v0, 4, 4, 39);
}

#define ROGUE_CR_PM_VHEAP_TABLE_length         2
#define ROGUE_CR_PM_VHEAP_TABLE_header          \


/* Helper macros for base_addr */
#define ROGUE_CR_PM_VHEAP_TABLE_BASE_ADDR_ALIGNMENT 16

struct ROGUE_CR_PM_VHEAP_TABLE {
    __pvr_address_type                   base_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PM_VHEAP_TABLE_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_PM_VHEAP_TABLE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->base_addr, 4, 4, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PM_VHEAP_TABLE_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_PM_VHEAP_TABLE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->base_addr = __pvr_address_unpack(v0, 4, 4, 39);
}

#define ROGUE_CR_PM_MLIST0_BASE_length         2
#define ROGUE_CR_PM_MLIST0_BASE_header          \


/* Helper macros for addr */
#define ROGUE_CR_PM_MLIST0_BASE_ADDR_ALIGNMENT   16

struct ROGUE_CR_PM_MLIST0_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PM_MLIST0_BASE_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_PM_MLIST0_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 4, 4, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PM_MLIST0_BASE_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_PM_MLIST0_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 4, 4, 39);
}

#define ROGUE_CR_VDM_CTRL_STREAM_BASE_length      2
#define ROGUE_CR_VDM_CTRL_STREAM_BASE_header    \


/* Helper macros for addr */
#define ROGUE_CR_VDM_CTRL_STREAM_BASE_ADDR_ALIGNMENT 4

struct ROGUE_CR_VDM_CTRL_STREAM_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CTRL_STREAM_BASE_pack(__attribute__((unused)) void * restrict dst,
                                   __attribute__((unused)) const struct ROGUE_CR_VDM_CTRL_STREAM_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 2, 2, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CTRL_STREAM_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                     __attribute__((unused)) struct ROGUE_CR_VDM_CTRL_STREAM_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 2, 2, 39);
}

#define ROGUE_CR_VDM_CALL_STACK_POINTER_length      2
#define ROGUE_CR_VDM_CALL_STACK_POINTER_header  \


/* Helper macros for addr */
#define ROGUE_CR_VDM_CALL_STACK_POINTER_ADDR_ALIGNMENT 8

struct ROGUE_CR_VDM_CALL_STACK_POINTER {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CALL_STACK_POINTER_pack(__attribute__((unused)) void * restrict dst,
                                     __attribute__((unused)) const struct ROGUE_CR_VDM_CALL_STACK_POINTER * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 3, 3, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CALL_STACK_POINTER_unpack(__attribute__((unused)) const void * restrict src,
                                       __attribute__((unused)) struct ROGUE_CR_VDM_CALL_STACK_POINTER * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 3, 3, 39);
}

#define ROGUE_CR_VDM_CONTEXT_STATE_BASE_length      2
#define ROGUE_CR_VDM_CONTEXT_STATE_BASE_header  \


/* Helper macros for addr */
#define ROGUE_CR_VDM_CONTEXT_STATE_BASE_ADDR_ALIGNMENT 16

struct ROGUE_CR_VDM_CONTEXT_STATE_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_STATE_BASE_pack(__attribute__((unused)) void * restrict dst,
                                     __attribute__((unused)) const struct ROGUE_CR_VDM_CONTEXT_STATE_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 4, 4, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_STATE_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                       __attribute__((unused)) struct ROGUE_CR_VDM_CONTEXT_STATE_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 4, 4, 39);
}

#define ROGUE_CR_VDM_CONTEXT_STORE_TASK0_length      2
#define ROGUE_CR_VDM_CONTEXT_STORE_TASK0_header \


struct ROGUE_CR_VDM_CONTEXT_STORE_TASK0 {
    uint32_t                             pds_state1;
    uint32_t                             pds_state0;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_STORE_TASK0_pack(__attribute__((unused)) void * restrict dst,
                                      __attribute__((unused)) const struct ROGUE_CR_VDM_CONTEXT_STORE_TASK0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->pds_state0, 0, 31);

    dw[1] =
      __pvr_uint(values->pds_state1, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_STORE_TASK0_unpack(__attribute__((unused)) const void * restrict src,
                                        __attribute__((unused)) struct ROGUE_CR_VDM_CONTEXT_STORE_TASK0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_state0 = __pvr_uint_unpack(dw[0], 0, 31);
    values->pds_state1 = __pvr_uint_unpack(dw[1], 0, 31);
}

#define ROGUE_CR_VDM_CONTEXT_STORE_TASK1_length      1
#define ROGUE_CR_VDM_CONTEXT_STORE_TASK1_header \


struct ROGUE_CR_VDM_CONTEXT_STORE_TASK1 {
    uint32_t                             pds_state2;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_STORE_TASK1_pack(__attribute__((unused)) void * restrict dst,
                                      __attribute__((unused)) const struct ROGUE_CR_VDM_CONTEXT_STORE_TASK1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->pds_state2, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_STORE_TASK1_unpack(__attribute__((unused)) const void * restrict src,
                                        __attribute__((unused)) struct ROGUE_CR_VDM_CONTEXT_STORE_TASK1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_state2 = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_CR_VDM_CONTEXT_STORE_TASK2_length      2
#define ROGUE_CR_VDM_CONTEXT_STORE_TASK2_header \


struct ROGUE_CR_VDM_CONTEXT_STORE_TASK2 {
    uint32_t                             stream_out2;
    uint32_t                             stream_out1;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_STORE_TASK2_pack(__attribute__((unused)) void * restrict dst,
                                      __attribute__((unused)) const struct ROGUE_CR_VDM_CONTEXT_STORE_TASK2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->stream_out1, 0, 31);

    dw[1] =
      __pvr_uint(values->stream_out2, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_STORE_TASK2_unpack(__attribute__((unused)) const void * restrict src,
                                        __attribute__((unused)) struct ROGUE_CR_VDM_CONTEXT_STORE_TASK2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->stream_out1 = __pvr_uint_unpack(dw[0], 0, 31);
    values->stream_out2 = __pvr_uint_unpack(dw[1], 0, 31);
}

#define ROGUE_CR_VDM_CONTEXT_RESUME_TASK0_length      2
#define ROGUE_CR_VDM_CONTEXT_RESUME_TASK0_header\


struct ROGUE_CR_VDM_CONTEXT_RESUME_TASK0 {
    uint32_t                             pds_state1;
    uint32_t                             pds_state0;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_RESUME_TASK0_pack(__attribute__((unused)) void * restrict dst,
                                       __attribute__((unused)) const struct ROGUE_CR_VDM_CONTEXT_RESUME_TASK0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->pds_state0, 0, 31);

    dw[1] =
      __pvr_uint(values->pds_state1, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_RESUME_TASK0_unpack(__attribute__((unused)) const void * restrict src,
                                         __attribute__((unused)) struct ROGUE_CR_VDM_CONTEXT_RESUME_TASK0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_state0 = __pvr_uint_unpack(dw[0], 0, 31);
    values->pds_state1 = __pvr_uint_unpack(dw[1], 0, 31);
}

#define ROGUE_CR_VDM_CONTEXT_RESUME_TASK1_length      1
#define ROGUE_CR_VDM_CONTEXT_RESUME_TASK1_header\


struct ROGUE_CR_VDM_CONTEXT_RESUME_TASK1 {
    uint32_t                             pds_state2;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_RESUME_TASK1_pack(__attribute__((unused)) void * restrict dst,
                                       __attribute__((unused)) const struct ROGUE_CR_VDM_CONTEXT_RESUME_TASK1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->pds_state2, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_RESUME_TASK1_unpack(__attribute__((unused)) const void * restrict src,
                                         __attribute__((unused)) struct ROGUE_CR_VDM_CONTEXT_RESUME_TASK1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_state2 = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_CR_VDM_CONTEXT_RESUME_TASK2_length      2
#define ROGUE_CR_VDM_CONTEXT_RESUME_TASK2_header\


struct ROGUE_CR_VDM_CONTEXT_RESUME_TASK2 {
    uint32_t                             stream_out2;
    uint32_t                             stream_out1;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_RESUME_TASK2_pack(__attribute__((unused)) void * restrict dst,
                                       __attribute__((unused)) const struct ROGUE_CR_VDM_CONTEXT_RESUME_TASK2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->stream_out1, 0, 31);

    dw[1] =
      __pvr_uint(values->stream_out2, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_VDM_CONTEXT_RESUME_TASK2_unpack(__attribute__((unused)) const void * restrict src,
                                         __attribute__((unused)) struct ROGUE_CR_VDM_CONTEXT_RESUME_TASK2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->stream_out1 = __pvr_uint_unpack(dw[0], 0, 31);
    values->stream_out2 = __pvr_uint_unpack(dw[1], 0, 31);
}

#define ROGUE_CR_CDM_CONTEXT_STATE_BASE_length      2
#define ROGUE_CR_CDM_CONTEXT_STATE_BASE_header  \


/* Helper macros for addr */
#define ROGUE_CR_CDM_CONTEXT_STATE_BASE_ADDR_ALIGNMENT 16

struct ROGUE_CR_CDM_CONTEXT_STATE_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CONTEXT_STATE_BASE_pack(__attribute__((unused)) void * restrict dst,
                                     __attribute__((unused)) const struct ROGUE_CR_CDM_CONTEXT_STATE_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 4, 4, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CONTEXT_STATE_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                       __attribute__((unused)) struct ROGUE_CR_CDM_CONTEXT_STATE_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 4, 4, 39);
}

#define ROGUE_CR_CDM_CONTEXT_PDS0_length       2
#define ROGUE_CR_CDM_CONTEXT_PDS0_header        \


/* Helper macros for data_addr */
#define ROGUE_CR_CDM_CONTEXT_PDS0_DATA_ADDR_ALIGNMENT 16

/* Helper macros for code_addr */
#define ROGUE_CR_CDM_CONTEXT_PDS0_CODE_ADDR_ALIGNMENT 16

struct ROGUE_CR_CDM_CONTEXT_PDS0 {
    __pvr_address_type                   data_addr;
    __pvr_address_type                   code_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CONTEXT_PDS0_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_CR_CDM_CONTEXT_PDS0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->code_addr, 4, 4, 31) | 0;

    dw[1] = __pvr_address(values->data_addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CONTEXT_PDS0_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_CR_CDM_CONTEXT_PDS0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->code_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
    values->data_addr = __pvr_address_unpack(dw[1], 4, 4, 31);
}

#define ROGUE_CR_CDM_CTRL_STREAM_BASE_length      2
#define ROGUE_CR_CDM_CTRL_STREAM_BASE_header    \


/* Helper macros for addr */
#define ROGUE_CR_CDM_CTRL_STREAM_BASE_ADDR_ALIGNMENT 4

struct ROGUE_CR_CDM_CTRL_STREAM_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CTRL_STREAM_BASE_pack(__attribute__((unused)) void * restrict dst,
                                   __attribute__((unused)) const struct ROGUE_CR_CDM_CTRL_STREAM_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 2, 2, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CTRL_STREAM_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                     __attribute__((unused)) struct ROGUE_CR_CDM_CTRL_STREAM_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 2, 2, 39);
}

#define ROGUE_CR_CDM_CONTEXT_PDS1_length       1
#define ROGUE_CR_CDM_CONTEXT_PDS1_header        \


/* Helper macros for common_size */
#define ROGUE_CR_CDM_CONTEXT_PDS1_COMMON_SIZE_UNIT_SIZE 64

/* Helper macros for data_size */
#define ROGUE_CR_CDM_CONTEXT_PDS1_DATA_SIZE_UNIT_SIZE 16

struct ROGUE_CR_CDM_CONTEXT_PDS1 {
    bool                                 pds_seq_dep;
    bool                                 usc_seq_dep;
    bool                                 target;
    uint32_t                             unified_size;
    bool                                 common_shared;
    uint32_t                             common_size;
    uint32_t                             temp_size;
    uint32_t                             data_size;
    bool                                 fence;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CONTEXT_PDS1_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_CR_CDM_CONTEXT_PDS1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->pds_seq_dep, 29, 29) |
      __pvr_uint(values->usc_seq_dep, 28, 28) |
      __pvr_uint(values->target, 27, 27) |
      __pvr_uint(values->unified_size, 21, 26) |
      __pvr_uint(values->common_shared, 20, 20) |
      __pvr_uint(values->common_size, 11, 19) |
      __pvr_uint(values->temp_size, 7, 10) |
      __pvr_uint(values->data_size, 1, 6) |
      __pvr_uint(values->fence, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CONTEXT_PDS1_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_CR_CDM_CONTEXT_PDS1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_seq_dep = __pvr_uint_unpack(dw[0], 29, 29);
    values->usc_seq_dep = __pvr_uint_unpack(dw[0], 28, 28);
    values->target = __pvr_uint_unpack(dw[0], 27, 27);
    values->unified_size = __pvr_uint_unpack(dw[0], 21, 26);
    values->common_shared = __pvr_uint_unpack(dw[0], 20, 20);
    values->common_size = __pvr_uint_unpack(dw[0], 11, 19);
    values->temp_size = __pvr_uint_unpack(dw[0], 7, 10);
    values->data_size = __pvr_uint_unpack(dw[0], 1, 6);
    values->fence = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_CR_CDM_TERMINATE_PDS_length      2
#define ROGUE_CR_CDM_TERMINATE_PDS_header       \


/* Helper macros for data_addr */
#define ROGUE_CR_CDM_TERMINATE_PDS_DATA_ADDR_ALIGNMENT 16

/* Helper macros for code_addr */
#define ROGUE_CR_CDM_TERMINATE_PDS_CODE_ADDR_ALIGNMENT 16

struct ROGUE_CR_CDM_TERMINATE_PDS {
    __pvr_address_type                   data_addr;
    __pvr_address_type                   code_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_TERMINATE_PDS_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_CR_CDM_TERMINATE_PDS * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->code_addr, 4, 4, 31) | 0;

    dw[1] = __pvr_address(values->data_addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_TERMINATE_PDS_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_CR_CDM_TERMINATE_PDS * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->code_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
    values->data_addr = __pvr_address_unpack(dw[1], 4, 4, 31);
}

#define ROGUE_CR_CDM_TERMINATE_PDS1_length      1
#define ROGUE_CR_CDM_TERMINATE_PDS1_header      \


struct ROGUE_CR_CDM_TERMINATE_PDS1 {
    bool                                 pds_seq_dep;
    bool                                 usc_seq_dep;
    bool                                 target;
    uint32_t                             unified_size;
    bool                                 common_shared;
    uint32_t                             common_size;
    uint32_t                             temp_size;
    uint32_t                             data_size;
    bool                                 fence;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_TERMINATE_PDS1_pack(__attribute__((unused)) void * restrict dst,
                                 __attribute__((unused)) const struct ROGUE_CR_CDM_TERMINATE_PDS1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->pds_seq_dep, 29, 29) |
      __pvr_uint(values->usc_seq_dep, 28, 28) |
      __pvr_uint(values->target, 27, 27) |
      __pvr_uint(values->unified_size, 21, 26) |
      __pvr_uint(values->common_shared, 20, 20) |
      __pvr_uint(values->common_size, 11, 19) |
      __pvr_uint(values->temp_size, 7, 10) |
      __pvr_uint(values->data_size, 1, 6) |
      __pvr_uint(values->fence, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_TERMINATE_PDS1_unpack(__attribute__((unused)) const void * restrict src,
                                   __attribute__((unused)) struct ROGUE_CR_CDM_TERMINATE_PDS1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_seq_dep = __pvr_uint_unpack(dw[0], 29, 29);
    values->usc_seq_dep = __pvr_uint_unpack(dw[0], 28, 28);
    values->target = __pvr_uint_unpack(dw[0], 27, 27);
    values->unified_size = __pvr_uint_unpack(dw[0], 21, 26);
    values->common_shared = __pvr_uint_unpack(dw[0], 20, 20);
    values->common_size = __pvr_uint_unpack(dw[0], 11, 19);
    values->temp_size = __pvr_uint_unpack(dw[0], 7, 10);
    values->data_size = __pvr_uint_unpack(dw[0], 1, 6);
    values->fence = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_CR_CDM_CONTEXT_LOAD_PDS0_length      2
#define ROGUE_CR_CDM_CONTEXT_LOAD_PDS0_header   \


/* Helper macros for data_addr */
#define ROGUE_CR_CDM_CONTEXT_LOAD_PDS0_DATA_ADDR_ALIGNMENT 16

/* Helper macros for code_addr */
#define ROGUE_CR_CDM_CONTEXT_LOAD_PDS0_CODE_ADDR_ALIGNMENT 16

struct ROGUE_CR_CDM_CONTEXT_LOAD_PDS0 {
    __pvr_address_type                   data_addr;
    __pvr_address_type                   code_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CONTEXT_LOAD_PDS0_pack(__attribute__((unused)) void * restrict dst,
                                    __attribute__((unused)) const struct ROGUE_CR_CDM_CONTEXT_LOAD_PDS0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->code_addr, 4, 4, 31) | 0;

    dw[1] = __pvr_address(values->data_addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_CONTEXT_LOAD_PDS0_unpack(__attribute__((unused)) const void * restrict src,
                                      __attribute__((unused)) struct ROGUE_CR_CDM_CONTEXT_LOAD_PDS0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->code_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
    values->data_addr = __pvr_address_unpack(dw[1], 4, 4, 31);
}

#define ROGUE_CR_COMPUTE_CLUSTER_length        1
#define ROGUE_CR_COMPUTE_CLUSTER_header         \


struct ROGUE_CR_COMPUTE_CLUSTER {
    uint32_t                             mask;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_COMPUTE_CLUSTER_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_CR_COMPUTE_CLUSTER * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->mask, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_COMPUTE_CLUSTER_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_CR_COMPUTE_CLUSTER * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->mask = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_CR_CDM_ITEM_length               1
#define ROGUE_CR_CDM_ITEM_header                \


struct ROGUE_CR_CDM_ITEM {
    uint32_t                             mode;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_ITEM_pack(__attribute__((unused)) void * restrict dst,
                       __attribute__((unused)) const struct ROGUE_CR_CDM_ITEM * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->mode, 0, 1);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_CDM_ITEM_unpack(__attribute__((unused)) const void * restrict src,
                         __attribute__((unused)) struct ROGUE_CR_CDM_ITEM * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->mode = __pvr_uint_unpack(dw[0], 0, 1);
}

#define ROGUE_CR_PDS_CTRL0_length              1
#define ROGUE_CR_PDS_CTRL0_header               \


struct ROGUE_CR_PDS_CTRL0 {
/* if ROGUEXE is supported use: */
/* if COMPUTE is supported use: */
    uint32_t                             roguexe_max_num_cdm_tasks;
/* endif COMPUTE */
/* if NUM_RASTER_PIPES > 0 is supported use: */
    uint32_t                             roguexe_max_num_pdm_tasks;
/* endif NUM_RASTER_PIPES > 0 */
/* if NUM_TA > 0 is supported use: */
    uint32_t                             roguexe_max_num_vdm_tasks;
/* endif NUM_TA > 0 */
/* else ROGUEXE is not-supported use: */
/* if COMPUTE is supported use: */
    uint32_t                             max_num_cdm_tasks;
/* endif COMPUTE */
    uint32_t                             max_num_pdm_tasks;
    uint32_t                             max_num_vdm_tasks;
/* endif ROGUEXE */
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_CTRL0_pack(__attribute__((unused)) void * restrict dst,
                        __attribute__((unused)) const struct ROGUE_CR_PDS_CTRL0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->roguexe_max_num_cdm_tasks, 24, 31) |
      __pvr_uint(values->roguexe_max_num_pdm_tasks, 16, 23) |
      __pvr_uint(values->roguexe_max_num_vdm_tasks, 8, 15) |
      __pvr_uint(values->max_num_cdm_tasks, 24, 30) |
      __pvr_uint(values->max_num_pdm_tasks, 16, 22) |
      __pvr_uint(values->max_num_vdm_tasks, 8, 14);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_CTRL0_unpack(__attribute__((unused)) const void * restrict src,
                          __attribute__((unused)) struct ROGUE_CR_PDS_CTRL0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->roguexe_max_num_cdm_tasks = __pvr_uint_unpack(dw[0], 24, 31);
    values->roguexe_max_num_pdm_tasks = __pvr_uint_unpack(dw[0], 16, 23);
    values->roguexe_max_num_vdm_tasks = __pvr_uint_unpack(dw[0], 8, 15);
    values->max_num_cdm_tasks = __pvr_uint_unpack(dw[0], 24, 30);
    values->max_num_pdm_tasks = __pvr_uint_unpack(dw[0], 16, 22);
    values->max_num_vdm_tasks = __pvr_uint_unpack(dw[0], 8, 14);
}

#define ROGUE_CR_PDS_CTRL1_length              1
#define ROGUE_CR_PDS_CTRL1_header               \


struct ROGUE_CR_PDS_CTRL1 {
    bool                                 sm_overlap_enable;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_CTRL1_pack(__attribute__((unused)) void * restrict dst,
                        __attribute__((unused)) const struct ROGUE_CR_PDS_CTRL1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->sm_overlap_enable, 23, 23);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_CTRL1_unpack(__attribute__((unused)) const void * restrict src,
                          __attribute__((unused)) struct ROGUE_CR_PDS_CTRL1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->sm_overlap_enable = __pvr_uint_unpack(dw[0], 23, 23);
}

#define ROGUE_CR_EVENT_PIXEL_PDS_CODE_length      1
#define ROGUE_CR_EVENT_PIXEL_PDS_CODE_header    \


/* Helper macros for addr */
#define ROGUE_CR_EVENT_PIXEL_PDS_CODE_ADDR_ALIGNMENT 16

struct ROGUE_CR_EVENT_PIXEL_PDS_CODE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_EVENT_PIXEL_PDS_CODE_pack(__attribute__((unused)) void * restrict dst,
                                   __attribute__((unused)) const struct ROGUE_CR_EVENT_PIXEL_PDS_CODE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_EVENT_PIXEL_PDS_CODE_unpack(__attribute__((unused)) const void * restrict src,
                                     __attribute__((unused)) struct ROGUE_CR_EVENT_PIXEL_PDS_CODE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_CR_EVENT_PIXEL_PDS_DATA_length      1
#define ROGUE_CR_EVENT_PIXEL_PDS_DATA_header    \


/* Helper macros for addr */
#define ROGUE_CR_EVENT_PIXEL_PDS_DATA_ADDR_ALIGNMENT 16

struct ROGUE_CR_EVENT_PIXEL_PDS_DATA {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_EVENT_PIXEL_PDS_DATA_pack(__attribute__((unused)) void * restrict dst,
                                   __attribute__((unused)) const struct ROGUE_CR_EVENT_PIXEL_PDS_DATA * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_EVENT_PIXEL_PDS_DATA_unpack(__attribute__((unused)) const void * restrict src,
                                     __attribute__((unused)) struct ROGUE_CR_EVENT_PIXEL_PDS_DATA * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_CR_EVENT_PIXEL_PDS_INFO_length      1
#define ROGUE_CR_EVENT_PIXEL_PDS_INFO_header    \


/* Helper macros for usc_sr_size */
#define ROGUE_CR_EVENT_PIXEL_PDS_INFO_USC_SR_SIZE_UNIT_SIZE 16

/* Helper macros for temp_stride */
#define ROGUE_CR_EVENT_PIXEL_PDS_INFO_TEMP_STRIDE_UNIT_SIZE 4

/* Helper macros for const_size */
#define ROGUE_CR_EVENT_PIXEL_PDS_INFO_CONST_SIZE_UNIT_SIZE 4

struct ROGUE_CR_EVENT_PIXEL_PDS_INFO {
    uint32_t                             usc_sr_size;
    uint32_t                             temp_stride;
    uint32_t                             const_size;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_EVENT_PIXEL_PDS_INFO_pack(__attribute__((unused)) void * restrict dst,
                                   __attribute__((unused)) const struct ROGUE_CR_EVENT_PIXEL_PDS_INFO * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->usc_sr_size, 9, 14) |
      __pvr_uint(values->temp_stride, 5, 8) |
      __pvr_uint(values->const_size, 0, 4);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_EVENT_PIXEL_PDS_INFO_unpack(__attribute__((unused)) const void * restrict src,
                                     __attribute__((unused)) struct ROGUE_CR_EVENT_PIXEL_PDS_INFO * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->usc_sr_size = __pvr_uint_unpack(dw[0], 9, 14);
    values->temp_stride = __pvr_uint_unpack(dw[0], 5, 8);
    values->const_size = __pvr_uint_unpack(dw[0], 0, 4);
}

#define ROGUE_CR_PDS_BGRND0_BASE_length        2
#define ROGUE_CR_PDS_BGRND0_BASE_header         \


/* Helper macros for texunicode_addr */
#define ROGUE_CR_PDS_BGRND0_BASE_TEXUNICODE_ADDR_ALIGNMENT 16

/* Helper macros for shader_addr */
#define ROGUE_CR_PDS_BGRND0_BASE_SHADER_ADDR_ALIGNMENT 16

struct ROGUE_CR_PDS_BGRND0_BASE {
    __pvr_address_type                   texunicode_addr;
    __pvr_address_type                   shader_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_BGRND0_BASE_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_CR_PDS_BGRND0_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->shader_addr, 4, 4, 31) | 0;

    dw[1] = __pvr_address(values->texunicode_addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_BGRND0_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_CR_PDS_BGRND0_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->shader_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
    values->texunicode_addr = __pvr_address_unpack(dw[1], 4, 4, 31);
}

#define ROGUE_CR_PDS_BGRND1_BASE_length        2
#define ROGUE_CR_PDS_BGRND1_BASE_header         \


/* Helper macros for texturedata_addr */
#define ROGUE_CR_PDS_BGRND1_BASE_TEXTUREDATA_ADDR_ALIGNMENT 16

/* Helper macros for varying_addr */
#define ROGUE_CR_PDS_BGRND1_BASE_VARYING_ADDR_ALIGNMENT 16

struct ROGUE_CR_PDS_BGRND1_BASE {
    __pvr_address_type                   texturedata_addr;
    __pvr_address_type                   varying_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_BGRND1_BASE_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_CR_PDS_BGRND1_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->varying_addr, 4, 4, 31) | 0;

    dw[1] = __pvr_address(values->texturedata_addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_BGRND1_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_CR_PDS_BGRND1_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->varying_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
    values->texturedata_addr = __pvr_address_unpack(dw[1], 4, 4, 31);
}

#define ROGUE_CR_PDS_BGRND2_BASE_length        2
#define ROGUE_CR_PDS_BGRND2_BASE_header         \


/* Helper macros for uniformdata_addr */
#define ROGUE_CR_PDS_BGRND2_BASE_UNIFORMDATA_ADDR_ALIGNMENT 16

struct ROGUE_CR_PDS_BGRND2_BASE {
    __pvr_address_type                   uniformdata_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_BGRND2_BASE_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_CR_PDS_BGRND2_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->uniformdata_addr, 4, 4, 31) | 0;

    dw[1] = 0;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_BGRND2_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_CR_PDS_BGRND2_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->uniformdata_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_CR_PDS_BGRND3_SIZEINFO_length      2
#define ROGUE_CR_PDS_BGRND3_SIZEINFO_header     \


/* Helper macros for usc_sharedsize */
#define ROGUE_CR_PDS_BGRND3_SIZEINFO_USC_SHAREDSIZE_UNIT_SIZE 16

/* Helper macros for pds_uniformsize */
#define ROGUE_CR_PDS_BGRND3_SIZEINFO_PDS_UNIFORMSIZE_UNIT_SIZE 4

/* Helper macros for pds_texturestatesize */
#define ROGUE_CR_PDS_BGRND3_SIZEINFO_PDS_TEXTURESTATESIZE_UNIT_SIZE 4

/* Helper macros for pds_varyingsize */
#define ROGUE_CR_PDS_BGRND3_SIZEINFO_PDS_VARYINGSIZE_UNIT_SIZE 4

/* Helper macros for usc_varyingsize */
#define ROGUE_CR_PDS_BGRND3_SIZEINFO_USC_VARYINGSIZE_UNIT_SIZE 16

/* Helper macros for pds_tempsize */
#define ROGUE_CR_PDS_BGRND3_SIZEINFO_PDS_TEMPSIZE_UNIT_SIZE 4

struct ROGUE_CR_PDS_BGRND3_SIZEINFO {
    uint32_t                             usc_sharedsize;
    uint32_t                             pds_batchnum;
    uint32_t                             pds_uniformsize;
    uint32_t                             pds_texturestatesize;
    uint32_t                             pds_varyingsize;
    uint32_t                             usc_varyingsize;
    uint32_t                             pds_tempsize;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_BGRND3_SIZEINFO_pack(__attribute__((unused)) void * restrict dst,
                                  __attribute__((unused)) const struct ROGUE_CR_PDS_BGRND3_SIZEINFO * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->pds_uniformsize, 23, 31) |
      __pvr_uint(values->pds_texturestatesize, 16, 22) |
      __pvr_uint(values->pds_varyingsize, 10, 15) |
      __pvr_uint(values->usc_varyingsize, 4, 9) |
      __pvr_uint(values->pds_tempsize, 0, 3);

    dw[1] =
      __pvr_uint(values->usc_sharedsize, 23, 31) |
      __pvr_uint(values->pds_batchnum, 0, 13);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PDS_BGRND3_SIZEINFO_unpack(__attribute__((unused)) const void * restrict src,
                                    __attribute__((unused)) struct ROGUE_CR_PDS_BGRND3_SIZEINFO * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_uniformsize = __pvr_uint_unpack(dw[0], 23, 31);
    values->pds_texturestatesize = __pvr_uint_unpack(dw[0], 16, 22);
    values->pds_varyingsize = __pvr_uint_unpack(dw[0], 10, 15);
    values->usc_varyingsize = __pvr_uint_unpack(dw[0], 4, 9);
    values->pds_tempsize = __pvr_uint_unpack(dw[0], 0, 3);
    values->usc_sharedsize = __pvr_uint_unpack(dw[1], 23, 31);
    values->pds_batchnum = __pvr_uint_unpack(dw[1], 0, 13);
}

#define ROGUE_CR_TE_AA_length                  1
#define ROGUE_CR_TE_AA_header                   \


struct ROGUE_CR_TE_AA {
/* if SIMPLE_INTERNAL_PARAMETER_FORMAT && ISP_SAMPLES_PER_PIXEL > 2 is supported use: */
    bool                                 y2;
/* endif SIMPLE_INTERNAL_PARAMETER_FORMAT && ISP_SAMPLES_PER_PIXEL > 2 */
    bool                                 y;
    bool                                 x;
    bool                                 x2;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_AA_pack(__attribute__((unused)) void * restrict dst,
                    __attribute__((unused)) const struct ROGUE_CR_TE_AA * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->y2, 3, 3) |
      __pvr_uint(values->y, 2, 2) |
      __pvr_uint(values->x, 1, 1) |
      __pvr_uint(values->x2, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_AA_unpack(__attribute__((unused)) const void * restrict src,
                      __attribute__((unused)) struct ROGUE_CR_TE_AA * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->y2 = __pvr_uint_unpack(dw[0], 3, 3);
    values->y = __pvr_uint_unpack(dw[0], 2, 2);
    values->x = __pvr_uint_unpack(dw[0], 1, 1);
    values->x2 = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_CR_TE_MTILE1_length              1
#define ROGUE_CR_TE_MTILE1_header               \


struct ROGUE_CR_TE_MTILE1 {
    uint32_t                             x1;
    uint32_t                             x2;
    uint32_t                             x3;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_MTILE1_pack(__attribute__((unused)) void * restrict dst,
                        __attribute__((unused)) const struct ROGUE_CR_TE_MTILE1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->x1, 18, 26) |
      __pvr_uint(values->x2, 9, 17) |
      __pvr_uint(values->x3, 0, 8);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_MTILE1_unpack(__attribute__((unused)) const void * restrict src,
                          __attribute__((unused)) struct ROGUE_CR_TE_MTILE1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->x1 = __pvr_uint_unpack(dw[0], 18, 26);
    values->x2 = __pvr_uint_unpack(dw[0], 9, 17);
    values->x3 = __pvr_uint_unpack(dw[0], 0, 8);
}

#define ROGUE_CR_TE_MTILE2_length              1
#define ROGUE_CR_TE_MTILE2_header               \


struct ROGUE_CR_TE_MTILE2 {
    uint32_t                             y1;
    uint32_t                             y2;
    uint32_t                             y3;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_MTILE2_pack(__attribute__((unused)) void * restrict dst,
                        __attribute__((unused)) const struct ROGUE_CR_TE_MTILE2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->y1, 18, 26) |
      __pvr_uint(values->y2, 9, 17) |
      __pvr_uint(values->y3, 0, 8);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_MTILE2_unpack(__attribute__((unused)) const void * restrict src,
                          __attribute__((unused)) struct ROGUE_CR_TE_MTILE2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->y1 = __pvr_uint_unpack(dw[0], 18, 26);
    values->y2 = __pvr_uint_unpack(dw[0], 9, 17);
    values->y3 = __pvr_uint_unpack(dw[0], 0, 8);
}

#define ROGUE_CR_TE_SCREEN_length              1
#define ROGUE_CR_TE_SCREEN_header               \


struct ROGUE_CR_TE_SCREEN {
    uint32_t                             ymax;
    uint32_t                             xmax;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_SCREEN_pack(__attribute__((unused)) void * restrict dst,
                        __attribute__((unused)) const struct ROGUE_CR_TE_SCREEN * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->ymax, 12, 20) |
      __pvr_uint(values->xmax, 0, 8);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_SCREEN_unpack(__attribute__((unused)) const void * restrict src,
                          __attribute__((unused)) struct ROGUE_CR_TE_SCREEN * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->ymax = __pvr_uint_unpack(dw[0], 12, 20);
    values->xmax = __pvr_uint_unpack(dw[0], 0, 8);
}

#define ROGUE_CR_TE_PSG_length                 1
#define ROGUE_CR_TE_PSG_header                  \


/* Helper macros for region_stride */
#define ROGUE_CR_TE_PSG_REGION_STRIDE_UNIT_SIZE  4096

struct ROGUE_CR_TE_PSG {
/* if ROGUEXE is supported use: */
/* if TILE_REGION_PROTECTION is supported use: */
    uint32_t                             force_protect;
/* endif TILE_REGION_PROTECTION */
    uint32_t                             cs_size;
    bool                                 enable_pwr_gate_state;
/* endif ROGUEXE */
    bool                                 enable_context_state_restore;
    bool                                 zonlyrender;
    bool                                 completeonterminate;
    bool                                 cache_bypass;
    bool                                 forcenewstate;
    uint32_t                             region_stride;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_PSG_pack(__attribute__((unused)) void * restrict dst,
                     __attribute__((unused)) const struct ROGUE_CR_TE_PSG * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->force_protect, 22, 22) |
      __pvr_uint(values->cs_size, 21, 21) |
      __pvr_uint(values->enable_pwr_gate_state, 20, 20) |
      __pvr_uint(values->enable_context_state_restore, 19, 19) |
      __pvr_uint(values->zonlyrender, 18, 18) |
      __pvr_uint(values->completeonterminate, 17, 17) |
      __pvr_uint(values->cache_bypass, 14, 14) |
      __pvr_uint(values->forcenewstate, 13, 13) |
      __pvr_uint(values->region_stride, 0, 10);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_PSG_unpack(__attribute__((unused)) const void * restrict src,
                       __attribute__((unused)) struct ROGUE_CR_TE_PSG * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->force_protect = __pvr_uint_unpack(dw[0], 22, 22);
    values->cs_size = __pvr_uint_unpack(dw[0], 21, 21);
    values->enable_pwr_gate_state = __pvr_uint_unpack(dw[0], 20, 20);
    values->enable_context_state_restore = __pvr_uint_unpack(dw[0], 19, 19);
    values->zonlyrender = __pvr_uint_unpack(dw[0], 18, 18);
    values->completeonterminate = __pvr_uint_unpack(dw[0], 17, 17);
    values->cache_bypass = __pvr_uint_unpack(dw[0], 14, 14);
    values->forcenewstate = __pvr_uint_unpack(dw[0], 13, 13);
    values->region_stride = __pvr_uint_unpack(dw[0], 0, 10);
}

#define ROGUE_CR_TE_PSGREGION_ADDR_length      2
#define ROGUE_CR_TE_PSGREGION_ADDR_header       \


/* Helper macros for base */
#define ROGUE_CR_TE_PSGREGION_ADDR_BASE_ALIGNMENT 64

struct ROGUE_CR_TE_PSGREGION_ADDR {
    __pvr_address_type                   base;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_PSGREGION_ADDR_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_CR_TE_PSGREGION_ADDR * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->base, 6, 6, 33);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_PSGREGION_ADDR_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_CR_TE_PSGREGION_ADDR * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->base = __pvr_address_unpack(v0, 6, 6, 33);
}

#define ROGUE_CR_TE_TPC_ADDR_length            2
#define ROGUE_CR_TE_TPC_ADDR_header             \


/* Helper macros for base */
#define ROGUE_CR_TE_TPC_ADDR_BASE_ALIGNMENT      64

struct ROGUE_CR_TE_TPC_ADDR {
    __pvr_address_type                   base;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_TPC_ADDR_pack(__attribute__((unused)) void * restrict dst,
                          __attribute__((unused)) const struct ROGUE_CR_TE_TPC_ADDR * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->base, 6, 6, 33);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TE_TPC_ADDR_unpack(__attribute__((unused)) const void * restrict src,
                            __attribute__((unused)) struct ROGUE_CR_TE_TPC_ADDR * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->base = __pvr_address_unpack(v0, 6, 6, 33);
}

#define ROGUE_CR_PPP_MULTISAMPLECTL_length      2
#define ROGUE_CR_PPP_MULTISAMPLECTL_header      \


struct ROGUE_CR_PPP_MULTISAMPLECTL {
/* if MAX_MULTISAMPLE == 8 is supported use: */
    uint32_t                             msaa_y7;
    uint32_t                             msaa_x7;
    uint32_t                             msaa_y6;
    uint32_t                             msaa_x6;
    uint32_t                             msaa_y5;
    uint32_t                             msaa_x5;
    uint32_t                             msaa_y4;
    uint32_t                             msaa_x4;
/* endif MAX_MULTISAMPLE == 8 */
    uint32_t                             msaa_y3;
    uint32_t                             msaa_x3;
    uint32_t                             msaa_y2;
    uint32_t                             msaa_x2;
    uint32_t                             msaa_y1;
    uint32_t                             msaa_x1;
    uint32_t                             msaa_y0;
    uint32_t                             msaa_x0;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PPP_MULTISAMPLECTL_pack(__attribute__((unused)) void * restrict dst,
                                 __attribute__((unused)) const struct ROGUE_CR_PPP_MULTISAMPLECTL * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->msaa_y3, 28, 31) |
      __pvr_uint(values->msaa_x3, 24, 27) |
      __pvr_uint(values->msaa_y2, 20, 23) |
      __pvr_uint(values->msaa_x2, 16, 19) |
      __pvr_uint(values->msaa_y1, 12, 15) |
      __pvr_uint(values->msaa_x1, 8, 11) |
      __pvr_uint(values->msaa_y0, 4, 7) |
      __pvr_uint(values->msaa_x0, 0, 3);

    dw[1] =
      __pvr_uint(values->msaa_y7, 28, 31) |
      __pvr_uint(values->msaa_x7, 24, 27) |
      __pvr_uint(values->msaa_y6, 20, 23) |
      __pvr_uint(values->msaa_x6, 16, 19) |
      __pvr_uint(values->msaa_y5, 12, 15) |
      __pvr_uint(values->msaa_x5, 8, 11) |
      __pvr_uint(values->msaa_y4, 4, 7) |
      __pvr_uint(values->msaa_x4, 0, 3);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PPP_MULTISAMPLECTL_unpack(__attribute__((unused)) const void * restrict src,
                                   __attribute__((unused)) struct ROGUE_CR_PPP_MULTISAMPLECTL * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->msaa_y3 = __pvr_uint_unpack(dw[0], 28, 31);
    values->msaa_x3 = __pvr_uint_unpack(dw[0], 24, 27);
    values->msaa_y2 = __pvr_uint_unpack(dw[0], 20, 23);
    values->msaa_x2 = __pvr_uint_unpack(dw[0], 16, 19);
    values->msaa_y1 = __pvr_uint_unpack(dw[0], 12, 15);
    values->msaa_x1 = __pvr_uint_unpack(dw[0], 8, 11);
    values->msaa_y0 = __pvr_uint_unpack(dw[0], 4, 7);
    values->msaa_x0 = __pvr_uint_unpack(dw[0], 0, 3);
    values->msaa_y7 = __pvr_uint_unpack(dw[1], 28, 31);
    values->msaa_x7 = __pvr_uint_unpack(dw[1], 24, 27);
    values->msaa_y6 = __pvr_uint_unpack(dw[1], 20, 23);
    values->msaa_x6 = __pvr_uint_unpack(dw[1], 16, 19);
    values->msaa_y5 = __pvr_uint_unpack(dw[1], 12, 15);
    values->msaa_x5 = __pvr_uint_unpack(dw[1], 8, 11);
    values->msaa_y4 = __pvr_uint_unpack(dw[1], 4, 7);
    values->msaa_x4 = __pvr_uint_unpack(dw[1], 0, 3);
}

#define ROGUE_CR_PPP_CTRL_length               1
#define ROGUE_CR_PPP_CTRL_header                \


struct ROGUE_CR_PPP_CTRL {
    bool                                 vpt_scissor;
    uint32_t                             flush_mode;
    bool                                 bfcull_restrict_clip;
    uint32_t                             fixed_point_format;
    bool                                 default_point_size;
    bool                                 bfcull1_disable;
    bool                                 bfcull2_disable;
    bool                                 fccull_disable;
    bool                                 oscull_disable;
    bool                                 socull_disable;
    bool                                 wclampen;
    bool                                 opengl;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PPP_CTRL_pack(__attribute__((unused)) void * restrict dst,
                       __attribute__((unused)) const struct ROGUE_CR_PPP_CTRL * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->vpt_scissor, 12, 12) |
      __pvr_uint(values->flush_mode, 11, 11) |
      __pvr_uint(values->bfcull_restrict_clip, 10, 10) |
      __pvr_uint(values->fixed_point_format, 9, 9) |
      __pvr_uint(values->default_point_size, 8, 8) |
      __pvr_uint(values->bfcull1_disable, 7, 7) |
      __pvr_uint(values->bfcull2_disable, 6, 6) |
      __pvr_uint(values->fccull_disable, 5, 5) |
      __pvr_uint(values->oscull_disable, 4, 4) |
      __pvr_uint(values->socull_disable, 2, 2) |
      __pvr_uint(values->wclampen, 1, 1) |
      __pvr_uint(values->opengl, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PPP_CTRL_unpack(__attribute__((unused)) const void * restrict src,
                         __attribute__((unused)) struct ROGUE_CR_PPP_CTRL * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->vpt_scissor = __pvr_uint_unpack(dw[0], 12, 12);
    values->flush_mode = __pvr_uint_unpack(dw[0], 11, 11);
    values->bfcull_restrict_clip = __pvr_uint_unpack(dw[0], 10, 10);
    values->fixed_point_format = __pvr_uint_unpack(dw[0], 9, 9);
    values->default_point_size = __pvr_uint_unpack(dw[0], 8, 8);
    values->bfcull1_disable = __pvr_uint_unpack(dw[0], 7, 7);
    values->bfcull2_disable = __pvr_uint_unpack(dw[0], 6, 6);
    values->fccull_disable = __pvr_uint_unpack(dw[0], 5, 5);
    values->oscull_disable = __pvr_uint_unpack(dw[0], 4, 4);
    values->socull_disable = __pvr_uint_unpack(dw[0], 2, 2);
    values->wclampen = __pvr_uint_unpack(dw[0], 1, 1);
    values->opengl = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_CR_PPP_SCREEN_length             1
#define ROGUE_CR_PPP_SCREEN_header              \


struct ROGUE_CR_PPP_SCREEN {
    uint32_t                             pixymax;
    uint32_t                             pixxmax;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PPP_SCREEN_pack(__attribute__((unused)) void * restrict dst,
                         __attribute__((unused)) const struct ROGUE_CR_PPP_SCREEN * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->pixymax, 16, 30) |
      __pvr_uint(values->pixxmax, 0, 14);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PPP_SCREEN_unpack(__attribute__((unused)) const void * restrict src,
                           __attribute__((unused)) struct ROGUE_CR_PPP_SCREEN * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pixymax = __pvr_uint_unpack(dw[0], 16, 30);
    values->pixxmax = __pvr_uint_unpack(dw[0], 0, 14);
}

#define ROGUE_CR_TA_RTC_ADDR_length            2
#define ROGUE_CR_TA_RTC_ADDR_header             \


/* Helper macros for base */
#define ROGUE_CR_TA_RTC_ADDR_BASE_ALIGNMENT      64

struct ROGUE_CR_TA_RTC_ADDR {
    __pvr_address_type                   base;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TA_RTC_ADDR_pack(__attribute__((unused)) void * restrict dst,
                          __attribute__((unused)) const struct ROGUE_CR_TA_RTC_ADDR * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->base, 6, 6, 33);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TA_RTC_ADDR_unpack(__attribute__((unused)) const void * restrict src,
                            __attribute__((unused)) struct ROGUE_CR_TA_RTC_ADDR * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->base = __pvr_address_unpack(v0, 6, 6, 33);
}

#define ROGUE_CR_TA_CONTEXT_STATE_BASE_length      2
#define ROGUE_CR_TA_CONTEXT_STATE_BASE_header   \


/* Helper macros for addr */
#define ROGUE_CR_TA_CONTEXT_STATE_BASE_ADDR_ALIGNMENT 16

struct ROGUE_CR_TA_CONTEXT_STATE_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TA_CONTEXT_STATE_BASE_pack(__attribute__((unused)) void * restrict dst,
                                    __attribute__((unused)) const struct ROGUE_CR_TA_CONTEXT_STATE_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 4, 4, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TA_CONTEXT_STATE_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                      __attribute__((unused)) struct ROGUE_CR_TA_CONTEXT_STATE_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 4, 4, 39);
}

#define ROGUE_CR_ISP_RENDER_length             1
#define ROGUE_CR_ISP_RENDER_header              \


struct ROGUE_CR_ISP_RENDER {
    bool                                 disable_eomt;
    bool                                 resume;
    enum ROGUE_CR_DIR_TYPE               dir_type;
    enum ROGUE_CR_ISP_RENDER_MODE_TYPE   mode_type;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_RENDER_pack(__attribute__((unused)) void * restrict dst,
                         __attribute__((unused)) const struct ROGUE_CR_ISP_RENDER * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->disable_eomt, 5, 5) |
      __pvr_uint(values->resume, 4, 4) |
      __pvr_uint(values->dir_type, 2, 3) |
      __pvr_uint(values->mode_type, 0, 1);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_RENDER_unpack(__attribute__((unused)) const void * restrict src,
                           __attribute__((unused)) struct ROGUE_CR_ISP_RENDER * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->disable_eomt = __pvr_uint_unpack(dw[0], 5, 5);
    values->resume = __pvr_uint_unpack(dw[0], 4, 4);
    values->dir_type = __pvr_uint_unpack(dw[0], 2, 3);
    values->mode_type = __pvr_uint_unpack(dw[0], 0, 1);
}

#define ROGUE_CR_ISP_RENDER_ORIGIN_length      1
#define ROGUE_CR_ISP_RENDER_ORIGIN_header       \


struct ROGUE_CR_ISP_RENDER_ORIGIN {
    uint32_t                             x;
    uint32_t                             y;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_RENDER_ORIGIN_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_CR_ISP_RENDER_ORIGIN * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->x, 16, 25) |
      __pvr_uint(values->y, 0, 9);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_RENDER_ORIGIN_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_CR_ISP_RENDER_ORIGIN * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->x = __pvr_uint_unpack(dw[0], 16, 25);
    values->y = __pvr_uint_unpack(dw[0], 0, 9);
}

#define ROGUE_CR_ISP_MTILE_BASE_length         2
#define ROGUE_CR_ISP_MTILE_BASE_header          \


/* Helper macros for addr */
#define ROGUE_CR_ISP_MTILE_BASE_ADDR_ALIGNMENT   4

struct ROGUE_CR_ISP_MTILE_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_MTILE_BASE_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_ISP_MTILE_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 2, 2, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_MTILE_BASE_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_ISP_MTILE_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 2, 2, 39);
}

#define ROGUE_CR_ISP_MTILE_SIZE_length         1
#define ROGUE_CR_ISP_MTILE_SIZE_header          \


struct ROGUE_CR_ISP_MTILE_SIZE {
    uint32_t                             x;
    uint32_t                             y;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_MTILE_SIZE_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_ISP_MTILE_SIZE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->x, 16, 25) |
      __pvr_uint(values->y, 0, 9);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_MTILE_SIZE_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_ISP_MTILE_SIZE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->x = __pvr_uint_unpack(dw[0], 16, 25);
    values->y = __pvr_uint_unpack(dw[0], 0, 9);
}

#define ROGUE_CR_ISP_RGN_length                1
#define ROGUE_CR_ISP_RGN_header                 \


/* Helper macros for size */
#define ROGUE_CR_ISP_RGN_SIZE_UNIT_SIZE          4

struct ROGUE_CR_ISP_RGN {
    uint32_t                             size;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_RGN_pack(__attribute__((unused)) void * restrict dst,
                      __attribute__((unused)) const struct ROGUE_CR_ISP_RGN * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->size, 0, 23);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_RGN_unpack(__attribute__((unused)) const void * restrict src,
                        __attribute__((unused)) struct ROGUE_CR_ISP_RGN * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->size = __pvr_uint_unpack(dw[0], 0, 23);
}

#define ROGUE_CR_ISP_RGN_SIPF_length           1
#define ROGUE_CR_ISP_RGN_SIPF_header            \


/* Helper macros for cs_size_ipf_creq_pf */
#define ROGUE_CR_ISP_RGN_SIPF_CS_SIZE_IPF_CREQ_PF_MAX 31

/* Helper macros for size */
#define ROGUE_CR_ISP_RGN_SIPF_SIZE_UNIT_SIZE     1

struct ROGUE_CR_ISP_RGN_SIPF {
/* if IPF_CREQ_PF is supported use: */
    uint32_t                             cs_size_ipf_creq_pf;
/* endif IPF_CREQ_PF */
    uint32_t                             size;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_RGN_SIPF_pack(__attribute__((unused)) void * restrict dst,
                           __attribute__((unused)) const struct ROGUE_CR_ISP_RGN_SIPF * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cs_size_ipf_creq_pf, 24, 28) |
      __pvr_uint(values->size, 0, 23);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_RGN_SIPF_unpack(__attribute__((unused)) const void * restrict src,
                             __attribute__((unused)) struct ROGUE_CR_ISP_RGN_SIPF * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cs_size_ipf_creq_pf = __pvr_uint_unpack(dw[0], 24, 28);
    values->size = __pvr_uint_unpack(dw[0], 0, 23);
}

#define ROGUE_CR_ISP_BGOBJDEPTH_length         1
#define ROGUE_CR_ISP_BGOBJDEPTH_header          \


struct ROGUE_CR_ISP_BGOBJDEPTH {
    uint32_t                             value;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_BGOBJDEPTH_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_ISP_BGOBJDEPTH * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->value, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_BGOBJDEPTH_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_ISP_BGOBJDEPTH * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->value = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_CR_ISP_BGOBJVALS_length          1
#define ROGUE_CR_ISP_BGOBJVALS_header           \


struct ROGUE_CR_ISP_BGOBJVALS {
    bool                                 enablebgtag;
    bool                                 mask;
    uint32_t                             stencil;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_BGOBJVALS_pack(__attribute__((unused)) void * restrict dst,
                            __attribute__((unused)) const struct ROGUE_CR_ISP_BGOBJVALS * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->enablebgtag, 9, 9) |
      __pvr_uint(values->mask, 8, 8) |
      __pvr_uint(values->stencil, 0, 7);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_BGOBJVALS_unpack(__attribute__((unused)) const void * restrict src,
                              __attribute__((unused)) struct ROGUE_CR_ISP_BGOBJVALS * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->enablebgtag = __pvr_uint_unpack(dw[0], 9, 9);
    values->mask = __pvr_uint_unpack(dw[0], 8, 8);
    values->stencil = __pvr_uint_unpack(dw[0], 0, 7);
}

#define ROGUE_CR_ISP_AA_length                 1
#define ROGUE_CR_ISP_AA_header                  \
    .mode                                = ROGUE_CR_ISP_AA_MODE_TYPE_AA_NONE

struct ROGUE_CR_ISP_AA {
    enum ROGUE_CR_ISP_AA_MODE_TYPE       mode;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_AA_pack(__attribute__((unused)) void * restrict dst,
                     __attribute__((unused)) const struct ROGUE_CR_ISP_AA * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->mode, 0, 1);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_AA_unpack(__attribute__((unused)) const void * restrict src,
                       __attribute__((unused)) struct ROGUE_CR_ISP_AA * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->mode = __pvr_uint_unpack(dw[0], 0, 1);
}

#define ROGUE_CR_ISP_CTL_length                1
#define ROGUE_CR_ISP_CTL_header                 \


/* Helper macros for upass_start */
#define ROGUE_CR_ISP_CTL_UPASS_START_SIZE_MAX    15

struct ROGUE_CR_ISP_CTL {
    bool                                 skip_init_hdrs;
    bool                                 line_style;
    bool                                 line_style_pix;
    bool                                 pair_tiles_vert;
    bool                                 pair_tiles;
    bool                                 creq_buf_en;
    bool                                 tile_age_en;
    enum ROGUE_CR_MODE_TYPE              isp_sample_pos_mode;
    uint32_t                             num_tiles_per_usc;
    bool                                 dbias_is_int;
    bool                                 overlap_check_mode;
    bool                                 pt_upfront_depth_disable;
    bool                                 process_empty_tiles;
    bool                                 sample_pos;
    enum ROGUE_CR_PIPE_NUM               pipe_enable;
    uint32_t                             valid_id;
    uint32_t                             upass_start;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_CTL_pack(__attribute__((unused)) void * restrict dst,
                      __attribute__((unused)) const struct ROGUE_CR_ISP_CTL * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->skip_init_hdrs, 31, 31) |
      __pvr_uint(values->line_style, 30, 30) |
      __pvr_uint(values->line_style_pix, 29, 29) |
      __pvr_uint(values->pair_tiles_vert, 28, 28) |
      __pvr_uint(values->pair_tiles, 27, 27) |
      __pvr_uint(values->creq_buf_en, 26, 26) |
      __pvr_uint(values->tile_age_en, 25, 25) |
      __pvr_uint(values->isp_sample_pos_mode, 23, 24) |
      __pvr_uint(values->num_tiles_per_usc, 21, 22) |
      __pvr_uint(values->dbias_is_int, 20, 20) |
      __pvr_uint(values->overlap_check_mode, 19, 19) |
      __pvr_uint(values->pt_upfront_depth_disable, 18, 18) |
      __pvr_uint(values->process_empty_tiles, 17, 17) |
      __pvr_uint(values->sample_pos, 16, 16) |
      __pvr_uint(values->pipe_enable, 12, 15) |
      __pvr_uint(values->valid_id, 4, 9) |
      __pvr_uint(values->upass_start, 0, 3);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_CTL_unpack(__attribute__((unused)) const void * restrict src,
                        __attribute__((unused)) struct ROGUE_CR_ISP_CTL * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->skip_init_hdrs = __pvr_uint_unpack(dw[0], 31, 31);
    values->line_style = __pvr_uint_unpack(dw[0], 30, 30);
    values->line_style_pix = __pvr_uint_unpack(dw[0], 29, 29);
    values->pair_tiles_vert = __pvr_uint_unpack(dw[0], 28, 28);
    values->pair_tiles = __pvr_uint_unpack(dw[0], 27, 27);
    values->creq_buf_en = __pvr_uint_unpack(dw[0], 26, 26);
    values->tile_age_en = __pvr_uint_unpack(dw[0], 25, 25);
    values->isp_sample_pos_mode = __pvr_uint_unpack(dw[0], 23, 24);
    values->num_tiles_per_usc = __pvr_uint_unpack(dw[0], 21, 22);
    values->dbias_is_int = __pvr_uint_unpack(dw[0], 20, 20);
    values->overlap_check_mode = __pvr_uint_unpack(dw[0], 19, 19);
    values->pt_upfront_depth_disable = __pvr_uint_unpack(dw[0], 18, 18);
    values->process_empty_tiles = __pvr_uint_unpack(dw[0], 17, 17);
    values->sample_pos = __pvr_uint_unpack(dw[0], 16, 16);
    values->pipe_enable = __pvr_uint_unpack(dw[0], 12, 15);
    values->valid_id = __pvr_uint_unpack(dw[0], 4, 9);
    values->upass_start = __pvr_uint_unpack(dw[0], 0, 3);
}

#define ROGUE_CR_ISP_ZLSCTL_length             2
#define ROGUE_CR_ISP_ZLSCTL_header              \


struct ROGUE_CR_ISP_ZLSCTL {
    uint32_t                             zlsextent_y_s;
    uint32_t                             zlsextent_x_s;
    bool                                 stencil_extent_enable;
    uint32_t                             zlsextent_y_z;
    enum ROGUE_CR_ZLS_FORMAT_TYPE        zstoreformat;
    enum ROGUE_CR_ZLS_FORMAT_TYPE        zloadformat;
    bool                                 fb_storeen;
    bool                                 fb_loaden;
    bool                                 mstoreen;
    bool                                 zstoreen;
    bool                                 sstoreen;
    bool                                 storetwiddled;
    bool                                 mloaden;
    bool                                 zloaden;
    bool                                 sloaden;
    bool                                 loadtwiddled;
    uint32_t                             zlsextent_x_z;
    bool                                 forcezstore;
    bool                                 forcezload;
    bool                                 zonlyrender;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_ZLSCTL_pack(__attribute__((unused)) void * restrict dst,
                         __attribute__((unused)) const struct ROGUE_CR_ISP_ZLSCTL * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->zlsextent_y_z, 27, 36) |
      __pvr_uint(values->zlsextent_y_s, 48, 57) |
      __pvr_uint(values->zlsextent_x_s, 38, 47) |
      __pvr_uint(values->stencil_extent_enable, 37, 37) |
      __pvr_uint(values->zstoreformat, 25, 26) |
      __pvr_uint(values->zloadformat, 23, 24) |
      __pvr_uint(values->fb_storeen, 22, 22) |
      __pvr_uint(values->fb_loaden, 21, 21) |
      __pvr_uint(values->mstoreen, 20, 20) |
      __pvr_uint(values->zstoreen, 19, 19) |
      __pvr_uint(values->sstoreen, 18, 18) |
      __pvr_uint(values->storetwiddled, 17, 17) |
      __pvr_uint(values->mloaden, 16, 16) |
      __pvr_uint(values->zloaden, 15, 15) |
      __pvr_uint(values->sloaden, 14, 14) |
      __pvr_uint(values->loadtwiddled, 13, 13) |
      __pvr_uint(values->zlsextent_x_z, 3, 12) |
      __pvr_uint(values->forcezstore, 2, 2) |
      __pvr_uint(values->forcezload, 1, 1) |
      __pvr_uint(values->zonlyrender, 0, 0);
    dw[0] = v0;
    dw[1] = v0 >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_ZLSCTL_unpack(__attribute__((unused)) const void * restrict src,
                           __attribute__((unused)) struct ROGUE_CR_ISP_ZLSCTL * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->zlsextent_y_z = __pvr_uint_unpack(v0, 27, 36);
    values->zlsextent_y_s = __pvr_uint_unpack(v0, 48, 57);
    values->zlsextent_x_s = __pvr_uint_unpack(v0, 38, 47);
    values->stencil_extent_enable = __pvr_uint_unpack(v0, 37, 37);
    values->zstoreformat = __pvr_uint_unpack(v0, 25, 26);
    values->zloadformat = __pvr_uint_unpack(v0, 23, 24);
    values->fb_storeen = __pvr_uint_unpack(v0, 22, 22);
    values->fb_loaden = __pvr_uint_unpack(v0, 21, 21);
    values->mstoreen = __pvr_uint_unpack(v0, 20, 20);
    values->zstoreen = __pvr_uint_unpack(v0, 19, 19);
    values->sstoreen = __pvr_uint_unpack(v0, 18, 18);
    values->storetwiddled = __pvr_uint_unpack(v0, 17, 17);
    values->mloaden = __pvr_uint_unpack(v0, 16, 16);
    values->zloaden = __pvr_uint_unpack(v0, 15, 15);
    values->sloaden = __pvr_uint_unpack(v0, 14, 14);
    values->loadtwiddled = __pvr_uint_unpack(v0, 13, 13);
    values->zlsextent_x_z = __pvr_uint_unpack(v0, 3, 12);
    values->forcezstore = __pvr_uint_unpack(v0, 2, 2);
    values->forcezload = __pvr_uint_unpack(v0, 1, 1);
    values->zonlyrender = __pvr_uint_unpack(v0, 0, 0);
}

#define ROGUE_CR_ISP_ZLOAD_BASE_length         2
#define ROGUE_CR_ISP_ZLOAD_BASE_header          \


/* Helper macros for addr */
#define ROGUE_CR_ISP_ZLOAD_BASE_ADDR_ALIGNMENT   16

struct ROGUE_CR_ISP_ZLOAD_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_ZLOAD_BASE_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_ISP_ZLOAD_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 4, 4, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_ZLOAD_BASE_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_ISP_ZLOAD_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 4, 4, 39);
}

#define ROGUE_CR_ISP_STENCIL_LOAD_BASE_length      2
#define ROGUE_CR_ISP_STENCIL_LOAD_BASE_header   \


/* Helper macros for addr */
#define ROGUE_CR_ISP_STENCIL_LOAD_BASE_ADDR_ALIGNMENT 16

struct ROGUE_CR_ISP_STENCIL_LOAD_BASE {
    __pvr_address_type                   addr;
    bool                                 enable;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_STENCIL_LOAD_BASE_pack(__attribute__((unused)) void * restrict dst,
                                    __attribute__((unused)) const struct ROGUE_CR_ISP_STENCIL_LOAD_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->enable, 0, 0);
    const uint64_t v0_address =
      __pvr_address(values->addr, 4, 4, 39);
    dw[0] = v0_address | v0;
    dw[1] = ((v0_address) >> 32) | (v0 >> 32);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_STENCIL_LOAD_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                      __attribute__((unused)) struct ROGUE_CR_ISP_STENCIL_LOAD_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 4, 4, 39);
    values->enable = __pvr_uint_unpack(v0, 0, 0);
}

#define ROGUE_CR_ISP_SCISSOR_BASE_length       2
#define ROGUE_CR_ISP_SCISSOR_BASE_header        \


/* Helper macros for addr */
#define ROGUE_CR_ISP_SCISSOR_BASE_ADDR_ALIGNMENT 4

struct ROGUE_CR_ISP_SCISSOR_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_SCISSOR_BASE_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_CR_ISP_SCISSOR_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 2, 2, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_SCISSOR_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_CR_ISP_SCISSOR_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 2, 2, 39);
}

#define ROGUE_CR_ISP_DBIAS_BASE_length         2
#define ROGUE_CR_ISP_DBIAS_BASE_header          \


/* Helper macros for addr */
#define ROGUE_CR_ISP_DBIAS_BASE_ADDR_ALIGNMENT   4

struct ROGUE_CR_ISP_DBIAS_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_DBIAS_BASE_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_ISP_DBIAS_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 2, 2, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_DBIAS_BASE_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_ISP_DBIAS_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 2, 2, 39);
}

#define ROGUE_CR_ISP_OCLQRY_BASE_length        2
#define ROGUE_CR_ISP_OCLQRY_BASE_header         \


/* Helper macros for addr */
#define ROGUE_CR_ISP_OCLQRY_BASE_ADDR_ALIGNMENT  16

struct ROGUE_CR_ISP_OCLQRY_BASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_OCLQRY_BASE_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_CR_ISP_OCLQRY_BASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->addr, 4, 4, 39);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_OCLQRY_BASE_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_CR_ISP_OCLQRY_BASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->addr = __pvr_address_unpack(v0, 4, 4, 39);
}

#define ROGUE_CR_ISP_ZLS_PIXELS_length         1
#define ROGUE_CR_ISP_ZLS_PIXELS_header          \


struct ROGUE_CR_ISP_ZLS_PIXELS {
    uint32_t                             y;
    uint32_t                             x;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_ZLS_PIXELS_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_ISP_ZLS_PIXELS * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->y, 15, 29) |
      __pvr_uint(values->x, 0, 14);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_ISP_ZLS_PIXELS_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_ISP_ZLS_PIXELS * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->y = __pvr_uint_unpack(dw[0], 15, 29);
    values->x = __pvr_uint_unpack(dw[0], 0, 14);
}

#define ROGUE_CR_PBE_WORD0_MRT0_length         2
#define ROGUE_CR_PBE_WORD0_MRT0_header          \


/* Helper macros for linestride */
#define ROGUE_CR_PBE_WORD0_MRT0_LINESTRIDE_ALIGNMENT 2

struct ROGUE_CR_PBE_WORD0_MRT0 {
/* if TFBC is supported use: */
    enum ROGUE_CR_TFBC_LOSSY             tfbc_lossy;
/* endif TFBC */
    bool                                 x_rsrvd;
    uint32_t                             pair_tiles;
    enum ROGUE_CR_COMP_IADDR_TYPE        comp_iaddr_mode;
    bool                                 x_rsrvd2;
    bool                                 comp_cor_enable;
    bool                                 dither;
    bool                                 tilerelative;
    bool                                 downscale;
    enum ROGUE_CR_SIZE                   size_z;
    enum ROGUE_CR_ROTATION_TYPE          rotation;
    uint32_t                             linestride;
    enum ROGUE_CR_MEMLAYOUT              memlayout;
    enum ROGUE_CR_SWIZ                   swiz_chan3;
    enum ROGUE_CR_SWIZ                   swiz_chan2;
    enum ROGUE_CR_SWIZ                   swiz_chan1;
    enum ROGUE_CR_SWIZ                   swiz_chan0;
    uint32_t                             minclip_x;
    enum ROGUE_CR_TWOCOMP_GAMMA          twocomp_gamma;
    bool                                 gamma;
    bool                                 compression;
    enum ROGUE_CR_COMPRESS_SIZE          compress_size;
    bool                                 comp_indirect_table;
/* if PBE_YFLIP is supported use: */
    bool                                 y_flip;
/* endif PBE_YFLIP */
};

static inline __attribute__((always_inline)) void
ROGUE_CR_PBE_WORD0_MRT0_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_CR_PBE_WORD0_MRT0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->swiz_chan3, 29, 31) |
      __pvr_uint(values->swiz_chan2, 26, 28) |
      __pvr_uint(values->swiz_chan1, 23, 25) |
      __pvr_uint(values->swiz_chan0, 20, 22) |
      __pvr_uint(values->minclip_x, 6, 19) |
      __pvr_uint(values->twocomp_gamma, 5, 5) |
      __pvr_uint(values->gamma, 4, 4) |
      __pvr_uint(values->compression, 3, 3) |
      __pvr_uint(values->compress_size, 2, 2) |
      __pvr_uint(values->comp_indirect_table, 1, 1) |
      __pvr_uint(values->y_flip, 0, 0);

    dw[1] =
      __pvr_uint(values->tfbc_lossy, 30, 31) |
      __pvr_uint(values->x_rsrvd, 31, 31) |
      __pvr_uint(values->pair_tiles, 28, 28) |
      __pvr_uint(values->comp_iaddr_mode, 28, 28) |
      __pvr_uint(values->x_rsrvd2, 27, 27) |
      __pvr_uint(values->comp_cor_enable, 27, 27) |
      __pvr_uint(values->dither, 26, 26) |
      __pvr_uint(values->tilerelative, 25, 25) |
      __pvr_uint(values->downscale, 24, 24) |
      __pvr_uint(values->size_z, 20, 23) |
      __pvr_uint(values->rotation, 18, 19) |
      __pvr_uint(values->linestride, 2, 17) |
      __pvr_uint(values->memlayout, 0, 1);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_PBE_WORD0_MRT0_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_CR_PBE_WORD0_MRT0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->swiz_chan3 = __pvr_uint_unpack(dw[0], 29, 31);
    values->swiz_chan2 = __pvr_uint_unpack(dw[0], 26, 28);
    values->swiz_chan1 = __pvr_uint_unpack(dw[0], 23, 25);
    values->swiz_chan0 = __pvr_uint_unpack(dw[0], 20, 22);
    values->minclip_x = __pvr_uint_unpack(dw[0], 6, 19);
    values->twocomp_gamma = __pvr_uint_unpack(dw[0], 5, 5);
    values->gamma = __pvr_uint_unpack(dw[0], 4, 4);
    values->compression = __pvr_uint_unpack(dw[0], 3, 3);
    values->compress_size = __pvr_uint_unpack(dw[0], 2, 2);
    values->comp_indirect_table = __pvr_uint_unpack(dw[0], 1, 1);
    values->y_flip = __pvr_uint_unpack(dw[0], 0, 0);
    values->tfbc_lossy = __pvr_uint_unpack(dw[1], 30, 31);
    values->x_rsrvd = __pvr_uint_unpack(dw[1], 31, 31);
    values->pair_tiles = __pvr_uint_unpack(dw[1], 28, 28);
    values->comp_iaddr_mode = __pvr_uint_unpack(dw[1], 28, 28);
    values->x_rsrvd2 = __pvr_uint_unpack(dw[1], 27, 27);
    values->comp_cor_enable = __pvr_uint_unpack(dw[1], 27, 27);
    values->dither = __pvr_uint_unpack(dw[1], 26, 26);
    values->tilerelative = __pvr_uint_unpack(dw[1], 25, 25);
    values->downscale = __pvr_uint_unpack(dw[1], 24, 24);
    values->size_z = __pvr_uint_unpack(dw[1], 20, 23);
    values->rotation = __pvr_uint_unpack(dw[1], 18, 19);
    values->linestride = __pvr_uint_unpack(dw[1], 2, 17);
    values->memlayout = __pvr_uint_unpack(dw[1], 0, 1);
}

#define ROGUE_CR_FRAG_SCREEN_length            1
#define ROGUE_CR_FRAG_SCREEN_header             \


struct ROGUE_CR_FRAG_SCREEN {
    uint32_t                             ymax;
    uint32_t                             xmax;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_FRAG_SCREEN_pack(__attribute__((unused)) void * restrict dst,
                          __attribute__((unused)) const struct ROGUE_CR_FRAG_SCREEN * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->ymax, 16, 30) |
      __pvr_uint(values->xmax, 0, 14);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_FRAG_SCREEN_unpack(__attribute__((unused)) const void * restrict src,
                            __attribute__((unused)) struct ROGUE_CR_FRAG_SCREEN * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->ymax = __pvr_uint_unpack(dw[0], 16, 30);
    values->xmax = __pvr_uint_unpack(dw[0], 0, 14);
}

#define ROGUE_CR_TPU_length                    1
#define ROGUE_CR_TPU_header                     \


struct ROGUE_CR_TPU {
/* if PDSL0SIZE > 0 is supported use: */
    bool                                 mcu_pds_l0_off;
/* endif PDSL0SIZE > 0 */
/* if TPU_CEM_DATAMASTER_GLOBAL_REGISTERS is supported use: */
    bool                                 tag_cem_64_face_packing;
/* endif TPU_CEM_DATAMASTER_GLOBAL_REGISTERS */
    bool                                 tag_enable_mmu_prefetch;
    bool                                 tag_cem_4k_face_packing;
    bool                                 madd_config_l0off;
    bool                                 tag_cem_face_packing;
    bool                                 tag_cemedge_dontfilter;
/* if TPU_CEM_USG_NORMALISATION is supported use: */
    bool                                 tag_cemgrad_dontnegate;
/* endif TPU_CEM_USG_NORMALISATION */
    bool                                 madd_config_dxt35_transovr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_pack(__attribute__((unused)) void * restrict dst,
                  __attribute__((unused)) const struct ROGUE_CR_TPU * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->mcu_pds_l0_off, 8, 8) |
      __pvr_uint(values->tag_cem_64_face_packing, 7, 7) |
      __pvr_uint(values->tag_enable_mmu_prefetch, 6, 6) |
      __pvr_uint(values->tag_cem_4k_face_packing, 5, 5) |
      __pvr_uint(values->madd_config_l0off, 4, 4) |
      __pvr_uint(values->tag_cem_face_packing, 3, 3) |
      __pvr_uint(values->tag_cemedge_dontfilter, 2, 2) |
      __pvr_uint(values->tag_cemgrad_dontnegate, 1, 1) |
      __pvr_uint(values->madd_config_dxt35_transovr, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_unpack(__attribute__((unused)) const void * restrict src,
                    __attribute__((unused)) struct ROGUE_CR_TPU * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->mcu_pds_l0_off = __pvr_uint_unpack(dw[0], 8, 8);
    values->tag_cem_64_face_packing = __pvr_uint_unpack(dw[0], 7, 7);
    values->tag_enable_mmu_prefetch = __pvr_uint_unpack(dw[0], 6, 6);
    values->tag_cem_4k_face_packing = __pvr_uint_unpack(dw[0], 5, 5);
    values->madd_config_l0off = __pvr_uint_unpack(dw[0], 4, 4);
    values->tag_cem_face_packing = __pvr_uint_unpack(dw[0], 3, 3);
    values->tag_cemedge_dontfilter = __pvr_uint_unpack(dw[0], 2, 2);
    values->tag_cemgrad_dontnegate = __pvr_uint_unpack(dw[0], 1, 1);
    values->madd_config_dxt35_transovr = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_CR_TPU_BORDER_COLOUR_TABLE_PDM_length      2
#define ROGUE_CR_TPU_BORDER_COLOUR_TABLE_PDM_header\


/* Helper macros for border_colour_table_address */
#define ROGUE_CR_TPU_BORDER_COLOUR_TABLE_PDM_BORDER_COLOUR_TABLE_ADDRESS_ALIGNMENT 4

struct ROGUE_CR_TPU_BORDER_COLOUR_TABLE_PDM {
    __pvr_address_type                   border_colour_table_address;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_BORDER_COLOUR_TABLE_PDM_pack(__attribute__((unused)) void * restrict dst,
                                          __attribute__((unused)) const struct ROGUE_CR_TPU_BORDER_COLOUR_TABLE_PDM * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->border_colour_table_address, 2, 0, 37);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_BORDER_COLOUR_TABLE_PDM_unpack(__attribute__((unused)) const void * restrict src,
                                            __attribute__((unused)) struct ROGUE_CR_TPU_BORDER_COLOUR_TABLE_PDM * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->border_colour_table_address = __pvr_address_unpack(v0, 2, 0, 37);
}

#define ROGUE_CR_TPU_BORDER_COLOUR_TABLE_VDM_length      2
#define ROGUE_CR_TPU_BORDER_COLOUR_TABLE_VDM_header\


/* Helper macros for border_colour_table_address */
#define ROGUE_CR_TPU_BORDER_COLOUR_TABLE_VDM_BORDER_COLOUR_TABLE_ADDRESS_ALIGNMENT 4

struct ROGUE_CR_TPU_BORDER_COLOUR_TABLE_VDM {
    __pvr_address_type                   border_colour_table_address;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_BORDER_COLOUR_TABLE_VDM_pack(__attribute__((unused)) void * restrict dst,
                                          __attribute__((unused)) const struct ROGUE_CR_TPU_BORDER_COLOUR_TABLE_VDM * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->border_colour_table_address, 2, 0, 37);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_BORDER_COLOUR_TABLE_VDM_unpack(__attribute__((unused)) const void * restrict src,
                                            __attribute__((unused)) struct ROGUE_CR_TPU_BORDER_COLOUR_TABLE_VDM * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->border_colour_table_address = __pvr_address_unpack(v0, 2, 0, 37);
}

#define ROGUE_CR_TPU_BORDER_COLOUR_TABLE_CDM_length      2
#define ROGUE_CR_TPU_BORDER_COLOUR_TABLE_CDM_header\


/* Helper macros for border_colour_table_address */
#define ROGUE_CR_TPU_BORDER_COLOUR_TABLE_CDM_BORDER_COLOUR_TABLE_ADDRESS_ALIGNMENT 4

struct ROGUE_CR_TPU_BORDER_COLOUR_TABLE_CDM {
    __pvr_address_type                   border_colour_table_address;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_BORDER_COLOUR_TABLE_CDM_pack(__attribute__((unused)) void * restrict dst,
                                          __attribute__((unused)) const struct ROGUE_CR_TPU_BORDER_COLOUR_TABLE_CDM * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0_address =
      __pvr_address(values->border_colour_table_address, 2, 0, 37);
    dw[0] = v0_address;
    dw[1] = v0_address >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_BORDER_COLOUR_TABLE_CDM_unpack(__attribute__((unused)) const void * restrict src,
                                            __attribute__((unused)) struct ROGUE_CR_TPU_BORDER_COLOUR_TABLE_CDM * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->border_colour_table_address = __pvr_address_unpack(v0, 2, 0, 37);
}

#define ROGUE_CR_TPU_TAG_CDM_CTRL_length       1
#define ROGUE_CR_TPU_TAG_CDM_CTRL_header        \


struct ROGUE_CR_TPU_TAG_CDM_CTRL {
    bool                                 af_ratio_truncate_to_integer;
    bool                                 af_ratio_truncate_to_half;
    bool                                 af_filtering_mode;
    bool                                 yuv_cam_invalidate;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_TAG_CDM_CTRL_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_CR_TPU_TAG_CDM_CTRL * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->af_ratio_truncate_to_integer, 3, 3) |
      __pvr_uint(values->af_ratio_truncate_to_half, 2, 2) |
      __pvr_uint(values->af_filtering_mode, 1, 1) |
      __pvr_uint(values->yuv_cam_invalidate, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_TPU_TAG_CDM_CTRL_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_CR_TPU_TAG_CDM_CTRL * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->af_ratio_truncate_to_integer = __pvr_uint_unpack(dw[0], 3, 3);
    values->af_ratio_truncate_to_half = __pvr_uint_unpack(dw[0], 2, 2);
    values->af_filtering_mode = __pvr_uint_unpack(dw[0], 1, 1);
    values->yuv_cam_invalidate = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_CR_USC_PIXEL_OUTPUT_CTRL_length      1
#define ROGUE_CR_USC_PIXEL_OUTPUT_CTRL_header   \


struct ROGUE_CR_USC_PIXEL_OUTPUT_CTRL {
    uint32_t                             partition_mask;
    bool                                 enable_4th_partition;
    enum ROGUE_CR_PIXEL_WIDTH            width;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_USC_PIXEL_OUTPUT_CTRL_pack(__attribute__((unused)) void * restrict dst,
                                    __attribute__((unused)) const struct ROGUE_CR_USC_PIXEL_OUTPUT_CTRL * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->partition_mask, 3, 20) |
      __pvr_uint(values->enable_4th_partition, 2, 2) |
      __pvr_uint(values->width, 0, 1);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_USC_PIXEL_OUTPUT_CTRL_unpack(__attribute__((unused)) const void * restrict src,
                                      __attribute__((unused)) struct ROGUE_CR_USC_PIXEL_OUTPUT_CTRL * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->partition_mask = __pvr_uint_unpack(dw[0], 3, 20);
    values->enable_4th_partition = __pvr_uint_unpack(dw[0], 2, 2);
    values->width = __pvr_uint_unpack(dw[0], 0, 1);
}

#define ROGUE_CR_USC_CLEAR_REGISTER_length      1
#define ROGUE_CR_USC_CLEAR_REGISTER_header      \


struct ROGUE_CR_USC_CLEAR_REGISTER {
    uint32_t                             val;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_USC_CLEAR_REGISTER_pack(__attribute__((unused)) void * restrict dst,
                                 __attribute__((unused)) const struct ROGUE_CR_USC_CLEAR_REGISTER * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->val, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_CR_USC_CLEAR_REGISTER_unpack(__attribute__((unused)) const void * restrict src,
                                   __attribute__((unused)) struct ROGUE_CR_USC_CLEAR_REGISTER * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->val = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_CR_FB_CDC_ZLS_length             2
#define ROGUE_CR_FB_CDC_ZLS_header              \


struct ROGUE_CR_FB_CDC_ZLS {
/* if !ROGUEXE is supported use: */
    bool                                 fbc_iaddr_mode;
    bool                                 fbdc_iaddr_mode;
/* endif !ROGUEXE */
    uint32_t                             fbdc_stencil_fmt;
    uint32_t                             fbdc_depth_fmt;
    uint32_t                             fbc_num_tiles;
    bool                                 fbc_iaddr;
    bool                                 fbdc_iaddr;
};

static inline __attribute__((always_inline)) void
ROGUE_CR_FB_CDC_ZLS_pack(__attribute__((unused)) void * restrict dst,
                         __attribute__((unused)) const struct ROGUE_CR_FB_CDC_ZLS * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->fbdc_depth_fmt, 27, 33) |
      __pvr_uint(values->fbc_iaddr_mode, 42, 42) |
      __pvr_uint(values->fbdc_iaddr_mode, 41, 41) |
      __pvr_uint(values->fbdc_stencil_fmt, 34, 40) |
      __pvr_uint(values->fbc_num_tiles, 4, 26) |
      __pvr_uint(values->fbc_iaddr, 1, 1) |
      __pvr_uint(values->fbdc_iaddr, 0, 0);
    dw[0] = v0;
    dw[1] = v0 >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_CR_FB_CDC_ZLS_unpack(__attribute__((unused)) const void * restrict src,
                           __attribute__((unused)) struct ROGUE_CR_FB_CDC_ZLS * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->fbdc_depth_fmt = __pvr_uint_unpack(v0, 27, 33);
    values->fbc_iaddr_mode = __pvr_uint_unpack(v0, 42, 42);
    values->fbdc_iaddr_mode = __pvr_uint_unpack(v0, 41, 41);
    values->fbdc_stencil_fmt = __pvr_uint_unpack(v0, 34, 40);
    values->fbc_num_tiles = __pvr_uint_unpack(v0, 4, 26);
    values->fbc_iaddr = __pvr_uint_unpack(v0, 1, 1);
    values->fbdc_iaddr = __pvr_uint_unpack(v0, 0, 0);
}

#endif /* CR_H */
