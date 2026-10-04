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

#ifndef PBESTATE_H
#define PBESTATE_H

#include "csbgen/pvr_packet_helpers.h"

#ifndef __OPENCL_VERSION__
#define __constant
#endif



enum ROGUE_PBESTATE_COMP_IADDR_TYPE {
    ROGUE_PBESTATE_COMP_IADDR_TYPE_INDIRECT_1TILE =      0,
    ROGUE_PBESTATE_COMP_IADDR_TYPE_INDIRECT_4TILE =      1,
};

static __constant const char *
ROGUE_PBESTATE_COMP_IADDR_TYPE_to_str(const enum ROGUE_PBESTATE_COMP_IADDR_TYPE value)
{
    switch (value) {
    case ROGUE_PBESTATE_COMP_IADDR_TYPE_INDIRECT_1TILE: return "INDIRECT_1TILE";
    case ROGUE_PBESTATE_COMP_IADDR_TYPE_INDIRECT_4TILE: return "INDIRECT_4TILE";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_COMPRESS_SIZE {
    ROGUE_PBESTATE_COMPRESS_SIZE_BLOCK_8X8 =      0,
    ROGUE_PBESTATE_COMPRESS_SIZE_BLOCK_16X4 =      1,
};

static __constant const char *
ROGUE_PBESTATE_COMPRESS_SIZE_to_str(const enum ROGUE_PBESTATE_COMPRESS_SIZE value)
{
    switch (value) {
    case ROGUE_PBESTATE_COMPRESS_SIZE_BLOCK_8X8: return "BLOCK_8X8";
    case ROGUE_PBESTATE_COMPRESS_SIZE_BLOCK_16X4: return "BLOCK_16X4";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_COMPRESS_SIZE_EXT {
    ROGUE_PBESTATE_COMPRESS_SIZE_EXT_BLOCK_8X8_16X4 =      0,
    ROGUE_PBESTATE_COMPRESS_SIZE_EXT_BLOCK_32X2_RSRVD =      1,
};

static __constant const char *
ROGUE_PBESTATE_COMPRESS_SIZE_EXT_to_str(const enum ROGUE_PBESTATE_COMPRESS_SIZE_EXT value)
{
    switch (value) {
    case ROGUE_PBESTATE_COMPRESS_SIZE_EXT_BLOCK_8X8_16X4: return "BLOCK_8X8_16X4";
    case ROGUE_PBESTATE_COMPRESS_SIZE_EXT_BLOCK_32X2_RSRVD: return "BLOCK_32X2_RSRVD";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_COMPRESSION {
    ROGUE_PBESTATE_COMPRESSION_DISABLED  =      0,
    ROGUE_PBESTATE_COMPRESSION_ENABLED   =      1,
};

static __constant const char *
ROGUE_PBESTATE_COMPRESSION_to_str(const enum ROGUE_PBESTATE_COMPRESSION value)
{
    switch (value) {
    case ROGUE_PBESTATE_COMPRESSION_DISABLED: return "DISABLED";
    case ROGUE_PBESTATE_COMPRESSION_ENABLED: return "ENABLED";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_LOSSY {
    ROGUE_PBESTATE_LOSSY_DISABLED        =      0,
    ROGUE_PBESTATE_LOSSY_ENABLED         =      1,
};

static __constant const char *
ROGUE_PBESTATE_LOSSY_to_str(const enum ROGUE_PBESTATE_LOSSY value)
{
    switch (value) {
    case ROGUE_PBESTATE_LOSSY_DISABLED: return "DISABLED";
    case ROGUE_PBESTATE_LOSSY_ENABLED: return "ENABLED";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_MEMLAYOUT {
    ROGUE_PBESTATE_MEMLAYOUT_LINEAR      =      0,
    ROGUE_PBESTATE_MEMLAYOUT_TWIDDLE_2D  =      1,
    ROGUE_PBESTATE_MEMLAYOUT_TWIDDLE_3D  =      2,
    ROGUE_PBESTATE_MEMLAYOUT_TILED       =      3,
};

static __constant const char *
ROGUE_PBESTATE_MEMLAYOUT_to_str(const enum ROGUE_PBESTATE_MEMLAYOUT value)
{
    switch (value) {
    case ROGUE_PBESTATE_MEMLAYOUT_LINEAR: return "LINEAR";
    case ROGUE_PBESTATE_MEMLAYOUT_TWIDDLE_2D: return "TWIDDLE_2D";
    case ROGUE_PBESTATE_MEMLAYOUT_TWIDDLE_3D: return "TWIDDLE_3D";
    case ROGUE_PBESTATE_MEMLAYOUT_TILED: return "TILED";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_PACKMODE {
    ROGUE_PBESTATE_PACKMODE_U8U8U8U8     =      0,
    ROGUE_PBESTATE_PACKMODE_S8S8S8S8     =      1,
    ROGUE_PBESTATE_PACKMODE_X8U8S8S8     =      2,
    ROGUE_PBESTATE_PACKMODE_X8S8S8U8     =      3,
    ROGUE_PBESTATE_PACKMODE_A1R5G5B5     =      4,
    ROGUE_PBESTATE_PACKMODE_R5G5B5A1     =      5,
    ROGUE_PBESTATE_PACKMODE_A4R4G4B4     =      6,
    ROGUE_PBESTATE_PACKMODE_A8R3G3B2     =      7,
    ROGUE_PBESTATE_PACKMODE_U16U16U16U16 =      8,
    ROGUE_PBESTATE_PACKMODE_S16S16S16S16 =      9,
    ROGUE_PBESTATE_PACKMODE_F16F16F16F16 =     10,
    ROGUE_PBESTATE_PACKMODE_U32U32U32U32 =     11,
    ROGUE_PBESTATE_PACKMODE_S32S32S32S32 =     12,
    ROGUE_PBESTATE_PACKMODE_F32F32F32F32 =     13,
    ROGUE_PBESTATE_PACKMODE_A2R10B10G10  =     14,
    ROGUE_PBESTATE_PACKMODE_R10B10G10A2  =     15,
    ROGUE_PBESTATE_PACKMODE_A2F10F10F10  =     16,
    ROGUE_PBESTATE_PACKMODE_F10F10F10A2  =     17,
    ROGUE_PBESTATE_PACKMODE_U8U8U8       =     18,
    ROGUE_PBESTATE_PACKMODE_S8S8S8       =     19,
    ROGUE_PBESTATE_PACKMODE_R5G6B5       =     20,
    ROGUE_PBESTATE_PACKMODE_R5SG5SB6     =     21,
    ROGUE_PBESTATE_PACKMODE_B6G5SR5S     =     22,
    ROGUE_PBESTATE_PACKMODE_U16U16U16    =     23,
    ROGUE_PBESTATE_PACKMODE_S16S16S16    =     24,
    ROGUE_PBESTATE_PACKMODE_F16F16F16    =     25,
    ROGUE_PBESTATE_PACKMODE_U32U32U32    =     26,
    ROGUE_PBESTATE_PACKMODE_S32S32S32    =     27,
    ROGUE_PBESTATE_PACKMODE_F11F11F10    =     28,
    ROGUE_PBESTATE_PACKMODE_F10F11F11    =     29,
    ROGUE_PBESTATE_PACKMODE_SE9995       =     30,
    ROGUE_PBESTATE_PACKMODE_F32F32F32    =     31,
    ROGUE_PBESTATE_PACKMODE_X24U8F32     =     32,
    ROGUE_PBESTATE_PACKMODE_X24X8F32     =     33,
    ROGUE_PBESTATE_PACKMODE_X24G8X32     =     34,
    ROGUE_PBESTATE_PACKMODE_U8U8         =     35,
    ROGUE_PBESTATE_PACKMODE_S8S8         =     36,
    ROGUE_PBESTATE_PACKMODE_U16U16       =     37,
    ROGUE_PBESTATE_PACKMODE_S16S16       =     38,
    ROGUE_PBESTATE_PACKMODE_F16F16       =     39,
    ROGUE_PBESTATE_PACKMODE_U32U32       =     40,
    ROGUE_PBESTATE_PACKMODE_S32S32       =     41,
    ROGUE_PBESTATE_PACKMODE_F32F32       =     42,
    ROGUE_PBESTATE_PACKMODE_U24ST8       =     43,
    ROGUE_PBESTATE_PACKMODE_ST8U24       =     44,
    ROGUE_PBESTATE_PACKMODE_X8U24        =     45,
    ROGUE_PBESTATE_PACKMODE_U8X24        =     46,
    ROGUE_PBESTATE_PACKMODE_U8           =     47,
    ROGUE_PBESTATE_PACKMODE_S8           =     48,
    ROGUE_PBESTATE_PACKMODE_U16          =     49,
    ROGUE_PBESTATE_PACKMODE_S16          =     50,
    ROGUE_PBESTATE_PACKMODE_F16          =     51,
    ROGUE_PBESTATE_PACKMODE_U32          =     52,
    ROGUE_PBESTATE_PACKMODE_S32          =     53,
    ROGUE_PBESTATE_PACKMODE_F32          =     54,
    ROGUE_PBESTATE_PACKMODE_PBYTE        =     55,
    ROGUE_PBESTATE_PACKMODE_PWORD        =     56,
    ROGUE_PBESTATE_PACKMODE_ARGBV16_XR10 =     57,
    ROGUE_PBESTATE_PACKMODE_A2_XRBIAS_U10U10U10 =     58,
    ROGUE_PBESTATE_PACKMODE_YUV          =     59,
    ROGUE_PBESTATE_PACKMODE_U10U10U10_XRBIAS_A2 =     60,
    ROGUE_PBESTATE_PACKMODE_INVALID      = 4294967295,
};

static __constant const char *
ROGUE_PBESTATE_PACKMODE_to_str(const enum ROGUE_PBESTATE_PACKMODE value)
{
    switch (value) {
    case ROGUE_PBESTATE_PACKMODE_U8U8U8U8: return "U8U8U8U8";
    case ROGUE_PBESTATE_PACKMODE_S8S8S8S8: return "S8S8S8S8";
    case ROGUE_PBESTATE_PACKMODE_X8U8S8S8: return "X8U8S8S8";
    case ROGUE_PBESTATE_PACKMODE_X8S8S8U8: return "X8S8S8U8";
    case ROGUE_PBESTATE_PACKMODE_A1R5G5B5: return "A1R5G5B5";
    case ROGUE_PBESTATE_PACKMODE_R5G5B5A1: return "R5G5B5A1";
    case ROGUE_PBESTATE_PACKMODE_A4R4G4B4: return "A4R4G4B4";
    case ROGUE_PBESTATE_PACKMODE_A8R3G3B2: return "A8R3G3B2";
    case ROGUE_PBESTATE_PACKMODE_U16U16U16U16: return "U16U16U16U16";
    case ROGUE_PBESTATE_PACKMODE_S16S16S16S16: return "S16S16S16S16";
    case ROGUE_PBESTATE_PACKMODE_F16F16F16F16: return "F16F16F16F16";
    case ROGUE_PBESTATE_PACKMODE_U32U32U32U32: return "U32U32U32U32";
    case ROGUE_PBESTATE_PACKMODE_S32S32S32S32: return "S32S32S32S32";
    case ROGUE_PBESTATE_PACKMODE_F32F32F32F32: return "F32F32F32F32";
    case ROGUE_PBESTATE_PACKMODE_A2R10B10G10: return "A2R10B10G10";
    case ROGUE_PBESTATE_PACKMODE_R10B10G10A2: return "R10B10G10A2";
    case ROGUE_PBESTATE_PACKMODE_A2F10F10F10: return "A2F10F10F10";
    case ROGUE_PBESTATE_PACKMODE_F10F10F10A2: return "F10F10F10A2";
    case ROGUE_PBESTATE_PACKMODE_U8U8U8: return "U8U8U8";
    case ROGUE_PBESTATE_PACKMODE_S8S8S8: return "S8S8S8";
    case ROGUE_PBESTATE_PACKMODE_R5G6B5: return "R5G6B5";
    case ROGUE_PBESTATE_PACKMODE_R5SG5SB6: return "R5SG5SB6";
    case ROGUE_PBESTATE_PACKMODE_B6G5SR5S: return "B6G5SR5S";
    case ROGUE_PBESTATE_PACKMODE_U16U16U16: return "U16U16U16";
    case ROGUE_PBESTATE_PACKMODE_S16S16S16: return "S16S16S16";
    case ROGUE_PBESTATE_PACKMODE_F16F16F16: return "F16F16F16";
    case ROGUE_PBESTATE_PACKMODE_U32U32U32: return "U32U32U32";
    case ROGUE_PBESTATE_PACKMODE_S32S32S32: return "S32S32S32";
    case ROGUE_PBESTATE_PACKMODE_F11F11F10: return "F11F11F10";
    case ROGUE_PBESTATE_PACKMODE_F10F11F11: return "F10F11F11";
    case ROGUE_PBESTATE_PACKMODE_SE9995: return "SE9995";
    case ROGUE_PBESTATE_PACKMODE_F32F32F32: return "F32F32F32";
    case ROGUE_PBESTATE_PACKMODE_X24U8F32: return "X24U8F32";
    case ROGUE_PBESTATE_PACKMODE_X24X8F32: return "X24X8F32";
    case ROGUE_PBESTATE_PACKMODE_X24G8X32: return "X24G8X32";
    case ROGUE_PBESTATE_PACKMODE_U8U8: return "U8U8";
    case ROGUE_PBESTATE_PACKMODE_S8S8: return "S8S8";
    case ROGUE_PBESTATE_PACKMODE_U16U16: return "U16U16";
    case ROGUE_PBESTATE_PACKMODE_S16S16: return "S16S16";
    case ROGUE_PBESTATE_PACKMODE_F16F16: return "F16F16";
    case ROGUE_PBESTATE_PACKMODE_U32U32: return "U32U32";
    case ROGUE_PBESTATE_PACKMODE_S32S32: return "S32S32";
    case ROGUE_PBESTATE_PACKMODE_F32F32: return "F32F32";
    case ROGUE_PBESTATE_PACKMODE_U24ST8: return "U24ST8";
    case ROGUE_PBESTATE_PACKMODE_ST8U24: return "ST8U24";
    case ROGUE_PBESTATE_PACKMODE_X8U24: return "X8U24";
    case ROGUE_PBESTATE_PACKMODE_U8X24: return "U8X24";
    case ROGUE_PBESTATE_PACKMODE_U8: return "U8";
    case ROGUE_PBESTATE_PACKMODE_S8: return "S8";
    case ROGUE_PBESTATE_PACKMODE_U16: return "U16";
    case ROGUE_PBESTATE_PACKMODE_S16: return "S16";
    case ROGUE_PBESTATE_PACKMODE_F16: return "F16";
    case ROGUE_PBESTATE_PACKMODE_U32: return "U32";
    case ROGUE_PBESTATE_PACKMODE_S32: return "S32";
    case ROGUE_PBESTATE_PACKMODE_F32: return "F32";
    case ROGUE_PBESTATE_PACKMODE_PBYTE: return "PBYTE";
    case ROGUE_PBESTATE_PACKMODE_PWORD: return "PWORD";
    case ROGUE_PBESTATE_PACKMODE_ARGBV16_XR10: return "ARGBV16_XR10";
    case ROGUE_PBESTATE_PACKMODE_A2_XRBIAS_U10U10U10: return "A2_XRBIAS_U10U10U10";
    case ROGUE_PBESTATE_PACKMODE_YUV: return "YUV";
    case ROGUE_PBESTATE_PACKMODE_U10U10U10_XRBIAS_A2: return "U10U10U10_XRBIAS_A2";
    case ROGUE_PBESTATE_PACKMODE_INVALID: return "INVALID";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_PAIR_TILES {
    ROGUE_PBESTATE_PAIR_TILES_DISABLED   =      0,
    ROGUE_PBESTATE_PAIR_TILES_ENABLED    =      1,
};

static __constant const char *
ROGUE_PBESTATE_PAIR_TILES_to_str(const enum ROGUE_PBESTATE_PAIR_TILES value)
{
    switch (value) {
    case ROGUE_PBESTATE_PAIR_TILES_DISABLED: return "DISABLED";
    case ROGUE_PBESTATE_PAIR_TILES_ENABLED: return "ENABLED";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_ROTATION_TYPE {
    ROGUE_PBESTATE_ROTATION_TYPE_0_DEG   =      0,
    ROGUE_PBESTATE_ROTATION_TYPE_90_DEG  =      1,
    ROGUE_PBESTATE_ROTATION_TYPE_180_DEG =      2,
    ROGUE_PBESTATE_ROTATION_TYPE_270_DEG =      3,
};

static __constant const char *
ROGUE_PBESTATE_ROTATION_TYPE_to_str(const enum ROGUE_PBESTATE_ROTATION_TYPE value)
{
    switch (value) {
    case ROGUE_PBESTATE_ROTATION_TYPE_0_DEG: return "_0_DEG";
    case ROGUE_PBESTATE_ROTATION_TYPE_90_DEG: return "_90_DEG";
    case ROGUE_PBESTATE_ROTATION_TYPE_180_DEG: return "_180_DEG";
    case ROGUE_PBESTATE_ROTATION_TYPE_270_DEG: return "_270_DEG";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_SIZE {
    ROGUE_PBESTATE_SIZE_1_PIXEL          =      0,
    ROGUE_PBESTATE_SIZE_2_PIXEL          =      1,
    ROGUE_PBESTATE_SIZE_4_PIXEL          =      2,
    ROGUE_PBESTATE_SIZE_8_PIXEL          =      3,
    ROGUE_PBESTATE_SIZE_16_PIXEL         =      4,
    ROGUE_PBESTATE_SIZE_32_PIXEL         =      5,
    ROGUE_PBESTATE_SIZE_64_PIXEL         =      6,
    ROGUE_PBESTATE_SIZE_128_PIXEL        =      7,
    ROGUE_PBESTATE_SIZE_256_PIXEL        =      8,
    ROGUE_PBESTATE_SIZE_512_PIXEL        =      9,
    ROGUE_PBESTATE_SIZE_1K_PIXEL         =     10,
    ROGUE_PBESTATE_SIZE_2K_PIXEL         =     11,
    ROGUE_PBESTATE_SIZE_4K_PIXEL         =     12,
    ROGUE_PBESTATE_SIZE_8K_PIXEL         =     13,
    ROGUE_PBESTATE_SIZE_16K_PIXEL        =     14,
};

static __constant const char *
ROGUE_PBESTATE_SIZE_to_str(const enum ROGUE_PBESTATE_SIZE value)
{
    switch (value) {
    case ROGUE_PBESTATE_SIZE_1_PIXEL: return "_1_PIXEL";
    case ROGUE_PBESTATE_SIZE_2_PIXEL: return "_2_PIXEL";
    case ROGUE_PBESTATE_SIZE_4_PIXEL: return "_4_PIXEL";
    case ROGUE_PBESTATE_SIZE_8_PIXEL: return "_8_PIXEL";
    case ROGUE_PBESTATE_SIZE_16_PIXEL: return "_16_PIXEL";
    case ROGUE_PBESTATE_SIZE_32_PIXEL: return "_32_PIXEL";
    case ROGUE_PBESTATE_SIZE_64_PIXEL: return "_64_PIXEL";
    case ROGUE_PBESTATE_SIZE_128_PIXEL: return "_128_PIXEL";
    case ROGUE_PBESTATE_SIZE_256_PIXEL: return "_256_PIXEL";
    case ROGUE_PBESTATE_SIZE_512_PIXEL: return "_512_PIXEL";
    case ROGUE_PBESTATE_SIZE_1K_PIXEL: return "_1K_PIXEL";
    case ROGUE_PBESTATE_SIZE_2K_PIXEL: return "_2K_PIXEL";
    case ROGUE_PBESTATE_SIZE_4K_PIXEL: return "_4K_PIXEL";
    case ROGUE_PBESTATE_SIZE_8K_PIXEL: return "_8K_PIXEL";
    case ROGUE_PBESTATE_SIZE_16K_PIXEL: return "_16K_PIXEL";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_SOURCE_FORMAT {
    ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL =      0,
    ROGUE_PBESTATE_SOURCE_FORMAT_8_PER_CHANNEL =      1,
};

static __constant const char *
ROGUE_PBESTATE_SOURCE_FORMAT_to_str(const enum ROGUE_PBESTATE_SOURCE_FORMAT value)
{
    switch (value) {
    case ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL: return "F16_PER_CHANNEL";
    case ROGUE_PBESTATE_SOURCE_FORMAT_8_PER_CHANNEL: return "_8_PER_CHANNEL";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_SOURCE_POS {
    ROGUE_PBESTATE_SOURCE_POS_START_BIT0 =      0,
    ROGUE_PBESTATE_SOURCE_POS_START_BIT32 =      1,
    ROGUE_PBESTATE_SOURCE_POS_START_BIT64 =      2,
    ROGUE_PBESTATE_SOURCE_POS_START_BIT96 =      3,
};

static __constant const char *
ROGUE_PBESTATE_SOURCE_POS_to_str(const enum ROGUE_PBESTATE_SOURCE_POS value)
{
    switch (value) {
    case ROGUE_PBESTATE_SOURCE_POS_START_BIT0: return "START_BIT0";
    case ROGUE_PBESTATE_SOURCE_POS_START_BIT32: return "START_BIT32";
    case ROGUE_PBESTATE_SOURCE_POS_START_BIT64: return "START_BIT64";
    case ROGUE_PBESTATE_SOURCE_POS_START_BIT96: return "START_BIT96";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_SWIZ {
    ROGUE_PBESTATE_SWIZ_SOURCE_CHAN0     =      0,
    ROGUE_PBESTATE_SWIZ_SOURCE_CHAN1     =      1,
    ROGUE_PBESTATE_SWIZ_SOURCE_CHAN2     =      2,
    ROGUE_PBESTATE_SWIZ_SOURCE_CHAN3     =      3,
    ROGUE_PBESTATE_SWIZ_ONE              =      4,
    ROGUE_PBESTATE_SWIZ_ZERO             =      5,
};

static __constant const char *
ROGUE_PBESTATE_SWIZ_to_str(const enum ROGUE_PBESTATE_SWIZ value)
{
    switch (value) {
    case ROGUE_PBESTATE_SWIZ_SOURCE_CHAN0: return "SOURCE_CHAN0";
    case ROGUE_PBESTATE_SWIZ_SOURCE_CHAN1: return "SOURCE_CHAN1";
    case ROGUE_PBESTATE_SWIZ_SOURCE_CHAN2: return "SOURCE_CHAN2";
    case ROGUE_PBESTATE_SWIZ_SOURCE_CHAN3: return "SOURCE_CHAN3";
    case ROGUE_PBESTATE_SWIZ_ONE: return "ONE";
    case ROGUE_PBESTATE_SWIZ_ZERO: return "ZERO";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_TFBC_LOSSY {
    ROGUE_PBESTATE_TFBC_LOSSY_LOSSLESS   =      0,
    ROGUE_PBESTATE_TFBC_LOSSY_LOSSY75    =      1,
    ROGUE_PBESTATE_TFBC_LOSSY_LOSSY50    =      2,
    ROGUE_PBESTATE_TFBC_LOSSY_LOSSY25    =      3,
};

static __constant const char *
ROGUE_PBESTATE_TFBC_LOSSY_to_str(const enum ROGUE_PBESTATE_TFBC_LOSSY value)
{
    switch (value) {
    case ROGUE_PBESTATE_TFBC_LOSSY_LOSSLESS: return "LOSSLESS";
    case ROGUE_PBESTATE_TFBC_LOSSY_LOSSY75: return "LOSSY75";
    case ROGUE_PBESTATE_TFBC_LOSSY_LOSSY50: return "LOSSY50";
    case ROGUE_PBESTATE_TFBC_LOSSY_LOSSY25: return "LOSSY25";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_TWOCOMP_GAMMA {
    ROGUE_PBESTATE_TWOCOMP_GAMMA_GAMMA_BOTTOM_CHANNEL =      0,
    ROGUE_PBESTATE_TWOCOMP_GAMMA_GAMMA_BOTH_CHANNELS =      1,
};

static __constant const char *
ROGUE_PBESTATE_TWOCOMP_GAMMA_to_str(const enum ROGUE_PBESTATE_TWOCOMP_GAMMA value)
{
    switch (value) {
    case ROGUE_PBESTATE_TWOCOMP_GAMMA_GAMMA_BOTTOM_CHANNEL: return "GAMMA_BOTTOM_CHANNEL";
    case ROGUE_PBESTATE_TWOCOMP_GAMMA_GAMMA_BOTH_CHANNELS: return "GAMMA_BOTH_CHANNELS";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_Y_FLIP {
    ROGUE_PBESTATE_Y_FLIP_DISABLED       =      0,
    ROGUE_PBESTATE_Y_FLIP_ENABLED        =      1,
};

static __constant const char *
ROGUE_PBESTATE_Y_FLIP_to_str(const enum ROGUE_PBESTATE_Y_FLIP value)
{
    switch (value) {
    case ROGUE_PBESTATE_Y_FLIP_DISABLED: return "DISABLED";
    case ROGUE_PBESTATE_Y_FLIP_ENABLED: return "ENABLED";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_YUV_DOWNSCALE {
    ROGUE_PBESTATE_YUV_DOWNSCALE_NO_DOWNSCALING =      0,
    ROGUE_PBESTATE_YUV_DOWNSCALE_ONE_SAMPLE_LEFT =      1,
    ROGUE_PBESTATE_YUV_DOWNSCALE_EMPTY   =      2,
    ROGUE_PBESTATE_YUV_DOWNSCALE_TWO_SAMPLES =      3,
    ROGUE_PBESTATE_YUV_DOWNSCALE_FOUR_SAMPLES =      4,
    ROGUE_PBESTATE_YUV_DOWNSCALE_ONE_SAMPLE_RIGHT =      5,
};

static __constant const char *
ROGUE_PBESTATE_YUV_DOWNSCALE_to_str(const enum ROGUE_PBESTATE_YUV_DOWNSCALE value)
{
    switch (value) {
    case ROGUE_PBESTATE_YUV_DOWNSCALE_NO_DOWNSCALING: return "NO_DOWNSCALING";
    case ROGUE_PBESTATE_YUV_DOWNSCALE_ONE_SAMPLE_LEFT: return "ONE_SAMPLE_LEFT";
    case ROGUE_PBESTATE_YUV_DOWNSCALE_EMPTY: return "EMPTY";
    case ROGUE_PBESTATE_YUV_DOWNSCALE_TWO_SAMPLES: return "TWO_SAMPLES";
    case ROGUE_PBESTATE_YUV_DOWNSCALE_FOUR_SAMPLES: return "FOUR_SAMPLES";
    case ROGUE_PBESTATE_YUV_DOWNSCALE_ONE_SAMPLE_RIGHT: return "ONE_SAMPLE_RIGHT";
    default: return NULL;
    }
}

enum ROGUE_PBESTATE_YUV_PMODE {
    ROGUE_PBESTATE_YUV_PMODE_UV8_420_2PLANE =      0,
    ROGUE_PBESTATE_YUV_PMODE_U8_420_3PLANE =      1,
    ROGUE_PBESTATE_YUV_PMODE_V8_420_3PLANE =      2,
    ROGUE_PBESTATE_YUV_PMODE_YUV8_422_1PLANE =      3,
    ROGUE_PBESTATE_YUV_PMODE_UV8_422_2PLANE =      4,
    ROGUE_PBESTATE_YUV_PMODE_UV8_444_2PLANE =      5,
    ROGUE_PBESTATE_YUV_PMODE_Y8_23PLANE  =      6,
    ROGUE_PBESTATE_YUV_PMODE_U8_444_3PLANE =      7,
    ROGUE_PBESTATE_YUV_PMODE_V8_444_3PLANE =      8,
    ROGUE_PBESTATE_YUV_PMODE_YUV10_444_1PLANE =      9,
    ROGUE_PBESTATE_YUV_PMODE_RESERVED_YUV1 =     10,
    ROGUE_PBESTATE_YUV_PMODE_RESERVED_YUV2 =     11,
    ROGUE_PBESTATE_YUV_PMODE_YUV10_422_1PLANE =     12,
    ROGUE_PBESTATE_YUV_PMODE_UV10_420_2PLANE =     13,
    ROGUE_PBESTATE_YUV_PMODE_Y10_23PLANE =     14,
    ROGUE_PBESTATE_YUV_PMODE_UV16_420_2PLANE =     15,
    ROGUE_PBESTATE_YUV_PMODE_UV16_422_2PLANE =     16,
    ROGUE_PBESTATE_YUV_PMODE_UV16_444_2PLANE =     17,
    ROGUE_PBESTATE_YUV_PMODE_Y16_23PLANE =     18,
    ROGUE_PBESTATE_YUV_PMODE_U16_444_3PLANE =     19,
    ROGUE_PBESTATE_YUV_PMODE_V16_444_3PLANE =     20,
};

static __constant const char *
ROGUE_PBESTATE_YUV_PMODE_to_str(const enum ROGUE_PBESTATE_YUV_PMODE value)
{
    switch (value) {
    case ROGUE_PBESTATE_YUV_PMODE_UV8_420_2PLANE: return "UV8_420_2PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_U8_420_3PLANE: return "U8_420_3PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_V8_420_3PLANE: return "V8_420_3PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_YUV8_422_1PLANE: return "YUV8_422_1PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_UV8_422_2PLANE: return "UV8_422_2PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_UV8_444_2PLANE: return "UV8_444_2PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_Y8_23PLANE: return "Y8_23PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_U8_444_3PLANE: return "U8_444_3PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_V8_444_3PLANE: return "V8_444_3PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_YUV10_444_1PLANE: return "YUV10_444_1PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_RESERVED_YUV1: return "RESERVED_YUV1";
    case ROGUE_PBESTATE_YUV_PMODE_RESERVED_YUV2: return "RESERVED_YUV2";
    case ROGUE_PBESTATE_YUV_PMODE_YUV10_422_1PLANE: return "YUV10_422_1PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_UV10_420_2PLANE: return "UV10_420_2PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_Y10_23PLANE: return "Y10_23PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_UV16_420_2PLANE: return "UV16_420_2PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_UV16_422_2PLANE: return "UV16_422_2PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_UV16_444_2PLANE: return "UV16_444_2PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_Y16_23PLANE: return "Y16_23PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_U16_444_3PLANE: return "U16_444_3PLANE";
    case ROGUE_PBESTATE_YUV_PMODE_V16_444_3PLANE: return "V16_444_3PLANE";
    default: return NULL;
    }
}

#define ROGUE_PBESTATE_STATE_WORD0_length      1
#define ROGUE_PBESTATE_STATE_WORD0_header       \


/* Helper macros for address_low */
#define ROGUE_PBESTATE_STATE_WORD0_ADDRESS_LOW_ALIGNMENT 4

struct ROGUE_PBESTATE_STATE_WORD0 {
    __pvr_address_type                   address_low;
};

static inline __attribute__((always_inline)) void
ROGUE_PBESTATE_STATE_WORD0_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_PBESTATE_STATE_WORD0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->address_low, 2, 0, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_PBESTATE_STATE_WORD0_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_PBESTATE_STATE_WORD0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->address_low = __pvr_address_unpack(dw[0], 2, 0, 31);
}

#define ROGUE_PBESTATE_STATE_WORD1_length      1
#define ROGUE_PBESTATE_STATE_WORD1_header       \


/* Helper macros for address_high */
#define ROGUE_PBESTATE_STATE_WORD1_ADDRESS_HIGH_ALIGNMENT 17179869184

struct ROGUE_PBESTATE_STATE_WORD1 {
/* if 8_OUTPUT_REGISTERS is supported use: */
    bool                                 source_pos_offset_128;
/* endif 8_OUTPUT_REGISTERS */
    enum ROGUE_PBESTATE_YUV_PMODE        yuv_pmode;
    enum ROGUE_PBESTATE_YUV_DOWNSCALE    yuv_downscale;
    enum ROGUE_PBESTATE_SOURCE_FORMAT    source_format;
    uint32_t                             mrt_index;
    enum ROGUE_PBESTATE_SOURCE_POS       source_pos;
    bool                                 norm;
    enum ROGUE_PBESTATE_PACKMODE         packmode;
    bool                                 emptytile;
    __pvr_address_type                   address_high;
};

static inline __attribute__((always_inline)) void
ROGUE_PBESTATE_STATE_WORD1_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_PBESTATE_STATE_WORD1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint32_t v0 =
      __pvr_uint(values->source_pos_offset_128, 28, 28) |
      __pvr_uint(values->yuv_pmode, 23, 27) |
      __pvr_uint(values->yuv_downscale, 20, 22) |
      __pvr_uint(values->source_format, 19, 19) |
      __pvr_uint(values->mrt_index, 16, 18) |
      __pvr_uint(values->source_pos, 14, 15) |
      __pvr_uint(values->norm, 13, 13) |
      __pvr_uint(values->packmode, 7, 12) |
      __pvr_uint(values->emptytile, 6, 6);
    dw[0] = __pvr_address(values->address_high, 34, 0, 5) | v0;
}

static inline __attribute__((always_inline)) void
ROGUE_PBESTATE_STATE_WORD1_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_PBESTATE_STATE_WORD1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->source_pos_offset_128 = __pvr_uint_unpack(dw[0], 28, 28);
    values->yuv_pmode = __pvr_uint_unpack(dw[0], 23, 27);
    values->yuv_downscale = __pvr_uint_unpack(dw[0], 20, 22);
    values->source_format = __pvr_uint_unpack(dw[0], 19, 19);
    values->mrt_index = __pvr_uint_unpack(dw[0], 16, 18);
    values->source_pos = __pvr_uint_unpack(dw[0], 14, 15);
    values->norm = __pvr_uint_unpack(dw[0], 13, 13);
    values->packmode = __pvr_uint_unpack(dw[0], 7, 12);
    values->emptytile = __pvr_uint_unpack(dw[0], 6, 6);
    values->address_high = __pvr_address_unpack(dw[0], 34, 0, 5);
}

#define ROGUE_PBESTATE_REG_WORD0_length        2
#define ROGUE_PBESTATE_REG_WORD0_header         \


/* Helper macros for tfbc_lossy */
#define ROGUE_PBESTATE_REG_WORD0_TFBC_LOSSY_LOSSY37_75_TFBC_LOSSY_37_PERCENT_ENABLED 1

/* Helper macros for linestride */
#define ROGUE_PBESTATE_REG_WORD0_LINESTRIDE_UNIT_SIZE 2

struct ROGUE_PBESTATE_REG_WORD0 {
    enum ROGUE_PBESTATE_TFBC_LOSSY       tfbc_lossy;
    enum ROGUE_PBESTATE_LOSSY            lossy;
    enum ROGUE_PBESTATE_COMPRESS_SIZE_EXT compress_size_ext;
    enum ROGUE_PBESTATE_COMP_IADDR_TYPE  comp_iaddr_mode;
    bool                                 comp_cor_enable;
    bool                                 dither;
    bool                                 tilerelative;
    bool                                 downscale;
    enum ROGUE_PBESTATE_SIZE             size_z;
    enum ROGUE_PBESTATE_ROTATION_TYPE    rotation;
    uint32_t                             linestride;
    enum ROGUE_PBESTATE_MEMLAYOUT        memlayout;
    enum ROGUE_PBESTATE_SWIZ             swiz_chan3;
    enum ROGUE_PBESTATE_SWIZ             swiz_chan2;
    enum ROGUE_PBESTATE_SWIZ             swiz_chan1;
    enum ROGUE_PBESTATE_SWIZ             swiz_chan0;
    uint32_t                             minclip_x;
    enum ROGUE_PBESTATE_TWOCOMP_GAMMA    twocomp_gamma;
    bool                                 gamma;
    enum ROGUE_PBESTATE_COMPRESSION      compression;
    enum ROGUE_PBESTATE_COMPRESS_SIZE    compress_size;
    bool                                 comp_indirect_table;
    enum ROGUE_PBESTATE_Y_FLIP           y_flip;
};

static inline __attribute__((always_inline)) void
ROGUE_PBESTATE_REG_WORD0_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_PBESTATE_REG_WORD0 * restrict values)
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
      __pvr_uint(values->lossy, 30, 30) |
      __pvr_uint(values->compress_size_ext, 29, 29) |
      __pvr_uint(values->comp_iaddr_mode, 28, 28) |
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
ROGUE_PBESTATE_REG_WORD0_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_PBESTATE_REG_WORD0 * restrict values)
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
    values->lossy = __pvr_uint_unpack(dw[1], 30, 30);
    values->compress_size_ext = __pvr_uint_unpack(dw[1], 29, 29);
    values->comp_iaddr_mode = __pvr_uint_unpack(dw[1], 28, 28);
    values->comp_cor_enable = __pvr_uint_unpack(dw[1], 27, 27);
    values->dither = __pvr_uint_unpack(dw[1], 26, 26);
    values->tilerelative = __pvr_uint_unpack(dw[1], 25, 25);
    values->downscale = __pvr_uint_unpack(dw[1], 24, 24);
    values->size_z = __pvr_uint_unpack(dw[1], 20, 23);
    values->rotation = __pvr_uint_unpack(dw[1], 18, 19);
    values->linestride = __pvr_uint_unpack(dw[1], 2, 17);
    values->memlayout = __pvr_uint_unpack(dw[1], 0, 1);
}

#define ROGUE_PBESTATE_REG_WORD1_length        2
#define ROGUE_PBESTATE_REG_WORD1_header         \


struct ROGUE_PBESTATE_REG_WORD1 {
    enum ROGUE_PBESTATE_SIZE             size_x;
    uint32_t                             minclip_y;
    uint32_t                             maxclip_x;
    enum ROGUE_PBESTATE_SIZE             size_y;
    uint32_t                             zslice;
    uint32_t                             maxclip_y;
};

static inline __attribute__((always_inline)) void
ROGUE_PBESTATE_REG_WORD1_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_PBESTATE_REG_WORD1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->size_y, 28, 31) |
      __pvr_uint(values->zslice, 14, 27) |
      __pvr_uint(values->maxclip_y, 0, 13);

    dw[1] =
      __pvr_uint(values->size_x, 28, 31) |
      __pvr_uint(values->minclip_y, 14, 27) |
      __pvr_uint(values->maxclip_x, 0, 13);
}

static inline __attribute__((always_inline)) void
ROGUE_PBESTATE_REG_WORD1_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_PBESTATE_REG_WORD1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->size_y = __pvr_uint_unpack(dw[0], 28, 31);
    values->zslice = __pvr_uint_unpack(dw[0], 14, 27);
    values->maxclip_y = __pvr_uint_unpack(dw[0], 0, 13);
    values->size_x = __pvr_uint_unpack(dw[1], 28, 31);
    values->minclip_y = __pvr_uint_unpack(dw[1], 14, 27);
    values->maxclip_x = __pvr_uint_unpack(dw[1], 0, 13);
}

#define ROGUE_PBESTATE_REG_WORD2_length        2
#define ROGUE_PBESTATE_REG_WORD2_header         \


/* Helper macros for pair_tiles */
#define ROGUE_PBESTATE_REG_WORD2_PAIR_TILES_SHIFT 46

struct ROGUE_PBESTATE_REG_WORD2 {
    enum ROGUE_PBESTATE_PAIR_TILES       pair_tiles;
    uint32_t                             surface_y_size;
    uint32_t                             sw_bytemask;
};

static inline __attribute__((always_inline)) void
ROGUE_PBESTATE_REG_WORD2_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_PBESTATE_REG_WORD2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->sw_bytemask, 0, 31);

    dw[1] =
      __pvr_uint(values->pair_tiles, 14, 14) |
      __pvr_uint(values->surface_y_size, 0, 13);
}

static inline __attribute__((always_inline)) void
ROGUE_PBESTATE_REG_WORD2_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_PBESTATE_REG_WORD2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->sw_bytemask = __pvr_uint_unpack(dw[0], 0, 31);
    values->pair_tiles = __pvr_uint_unpack(dw[1], 14, 14);
    values->surface_y_size = __pvr_uint_unpack(dw[1], 0, 13);
}

#endif /* PBESTATE_H */
