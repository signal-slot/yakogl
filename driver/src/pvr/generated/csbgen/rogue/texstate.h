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

#ifndef TEXSTATE_H
#define TEXSTATE_H

#include "csbgen/pvr_packet_helpers.h"

#ifndef __OPENCL_VERSION__
#define __constant
#endif



enum ROGUE_TEXSTATE_ADDRMODE {
    ROGUE_TEXSTATE_ADDRMODE_REPEAT       =      0,
    ROGUE_TEXSTATE_ADDRMODE_FLIP         =      1,
    ROGUE_TEXSTATE_ADDRMODE_CLAMP_TO_EDGE =      2,
    ROGUE_TEXSTATE_ADDRMODE_FLIP_ONCE_THEN_CLAMP =      3,
    ROGUE_TEXSTATE_ADDRMODE_CLAMP_TO_BORDER =      4,
    ROGUE_TEXSTATE_ADDRMODE_OGL_CLAMP    =      5,
};

static __constant const char *
ROGUE_TEXSTATE_ADDRMODE_to_str(const enum ROGUE_TEXSTATE_ADDRMODE value)
{
    switch (value) {
    case ROGUE_TEXSTATE_ADDRMODE_REPEAT: return "REPEAT";
    case ROGUE_TEXSTATE_ADDRMODE_FLIP: return "FLIP";
    case ROGUE_TEXSTATE_ADDRMODE_CLAMP_TO_EDGE: return "CLAMP_TO_EDGE";
    case ROGUE_TEXSTATE_ADDRMODE_FLIP_ONCE_THEN_CLAMP: return "FLIP_ONCE_THEN_CLAMP";
    case ROGUE_TEXSTATE_ADDRMODE_CLAMP_TO_BORDER: return "CLAMP_TO_BORDER";
    case ROGUE_TEXSTATE_ADDRMODE_OGL_CLAMP: return "OGL_CLAMP";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_ANISOCTL {
    ROGUE_TEXSTATE_ANISOCTL_DISABLED     =      0,
    ROGUE_TEXSTATE_ANISOCTL_X2           =      1,
    ROGUE_TEXSTATE_ANISOCTL_X4           =      2,
    ROGUE_TEXSTATE_ANISOCTL_X8           =      3,
    ROGUE_TEXSTATE_ANISOCTL_X16          =      4,
};

static __constant const char *
ROGUE_TEXSTATE_ANISOCTL_to_str(const enum ROGUE_TEXSTATE_ANISOCTL value)
{
    switch (value) {
    case ROGUE_TEXSTATE_ANISOCTL_DISABLED: return "DISABLED";
    case ROGUE_TEXSTATE_ANISOCTL_X2: return "X2";
    case ROGUE_TEXSTATE_ANISOCTL_X4: return "X4";
    case ROGUE_TEXSTATE_ANISOCTL_X8: return "X8";
    case ROGUE_TEXSTATE_ANISOCTL_X16: return "X16";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_CLAMP {
    ROGUE_TEXSTATE_CLAMP_MIN             =      0,
    ROGUE_TEXSTATE_CLAMP_MAX             =    959,
    ROGUE_TEXSTATE_CLAMP_INTEGER_BITS    =      4,
    ROGUE_TEXSTATE_CLAMP_FRACTIONAL_BITS =      6,
};

static __constant const char *
ROGUE_TEXSTATE_CLAMP_to_str(const enum ROGUE_TEXSTATE_CLAMP value)
{
    switch (value) {
    case ROGUE_TEXSTATE_CLAMP_MIN: return "MIN";
    case ROGUE_TEXSTATE_CLAMP_MAX: return "MAX";
    case ROGUE_TEXSTATE_CLAMP_INTEGER_BITS: return "INTEGER_BITS";
    case ROGUE_TEXSTATE_CLAMP_FRACTIONAL_BITS: return "FRACTIONAL_BITS";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_CMP_MODE {
    ROGUE_TEXSTATE_CMP_MODE_NEVER        =      0,
    ROGUE_TEXSTATE_CMP_MODE_LESS         =      1,
    ROGUE_TEXSTATE_CMP_MODE_EQUAL        =      2,
    ROGUE_TEXSTATE_CMP_MODE_LESSEQUAL    =      3,
    ROGUE_TEXSTATE_CMP_MODE_GREATER      =      4,
    ROGUE_TEXSTATE_CMP_MODE_NOTEQUAL     =      5,
    ROGUE_TEXSTATE_CMP_MODE_GREATEREQUAL =      6,
    ROGUE_TEXSTATE_CMP_MODE_ALWAYS       =      7,
};

static __constant const char *
ROGUE_TEXSTATE_CMP_MODE_to_str(const enum ROGUE_TEXSTATE_CMP_MODE value)
{
    switch (value) {
    case ROGUE_TEXSTATE_CMP_MODE_NEVER: return "NEVER";
    case ROGUE_TEXSTATE_CMP_MODE_LESS: return "LESS";
    case ROGUE_TEXSTATE_CMP_MODE_EQUAL: return "EQUAL";
    case ROGUE_TEXSTATE_CMP_MODE_LESSEQUAL: return "LESSEQUAL";
    case ROGUE_TEXSTATE_CMP_MODE_GREATER: return "GREATER";
    case ROGUE_TEXSTATE_CMP_MODE_NOTEQUAL: return "NOTEQUAL";
    case ROGUE_TEXSTATE_CMP_MODE_GREATEREQUAL: return "GREATEREQUAL";
    case ROGUE_TEXSTATE_CMP_MODE_ALWAYS: return "ALWAYS";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_COMPRESSION_LEVEL {
    ROGUE_TEXSTATE_COMPRESSION_LEVEL_LOSSLESS =      0,
    ROGUE_TEXSTATE_COMPRESSION_LEVEL_LOSSY_75 =      1,
    ROGUE_TEXSTATE_COMPRESSION_LEVEL_LOSSY_50 =      2,
    ROGUE_TEXSTATE_COMPRESSION_LEVEL_LOSSY_25 =      3,
};

static __constant const char *
ROGUE_TEXSTATE_COMPRESSION_LEVEL_to_str(const enum ROGUE_TEXSTATE_COMPRESSION_LEVEL value)
{
    switch (value) {
    case ROGUE_TEXSTATE_COMPRESSION_LEVEL_LOSSLESS: return "LOSSLESS";
    case ROGUE_TEXSTATE_COMPRESSION_LEVEL_LOSSY_75: return "LOSSY_75";
    case ROGUE_TEXSTATE_COMPRESSION_LEVEL_LOSSY_50: return "LOSSY_50";
    case ROGUE_TEXSTATE_COMPRESSION_LEVEL_LOSSY_25: return "LOSSY_25";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_COMPRESSION_MODE {
    ROGUE_TEXSTATE_COMPRESSION_MODE_NONE =      0,
    ROGUE_TEXSTATE_COMPRESSION_MODE_TPU  =      1,
    ROGUE_TEXSTATE_COMPRESSION_MODE_FB_DIRECT_8X8 =      2,
    ROGUE_TEXSTATE_COMPRESSION_MODE_FB_DIRECT_16X4 =      3,
    ROGUE_TEXSTATE_COMPRESSION_MODE_FB_INDIRECT_1TILE_8X8 =      4,
    ROGUE_TEXSTATE_COMPRESSION_MODE_FB_INDIRECT_1TILE_16X4 =      5,
    ROGUE_TEXSTATE_COMPRESSION_MODE_FB_INDIRECT_4TILE_8X8 =      6,
    ROGUE_TEXSTATE_COMPRESSION_MODE_FB_INDIRECT_4TILE_16X4 =      7,
};

static __constant const char *
ROGUE_TEXSTATE_COMPRESSION_MODE_to_str(const enum ROGUE_TEXSTATE_COMPRESSION_MODE value)
{
    switch (value) {
    case ROGUE_TEXSTATE_COMPRESSION_MODE_NONE: return "NONE";
    case ROGUE_TEXSTATE_COMPRESSION_MODE_TPU: return "TPU";
    case ROGUE_TEXSTATE_COMPRESSION_MODE_FB_DIRECT_8X8: return "FB_DIRECT_8X8";
    case ROGUE_TEXSTATE_COMPRESSION_MODE_FB_DIRECT_16X4: return "FB_DIRECT_16X4";
    case ROGUE_TEXSTATE_COMPRESSION_MODE_FB_INDIRECT_1TILE_8X8: return "FB_INDIRECT_1TILE_8X8";
    case ROGUE_TEXSTATE_COMPRESSION_MODE_FB_INDIRECT_1TILE_16X4: return "FB_INDIRECT_1TILE_16X4";
    case ROGUE_TEXSTATE_COMPRESSION_MODE_FB_INDIRECT_4TILE_8X8: return "FB_INDIRECT_4TILE_8X8";
    case ROGUE_TEXSTATE_COMPRESSION_MODE_FB_INDIRECT_4TILE_16X4: return "FB_INDIRECT_4TILE_16X4";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_DADJUST {
    ROGUE_TEXSTATE_DADJUST_MIN_UINT      =      0,
    ROGUE_TEXSTATE_DADJUST_ZERO_UINT     =   4095,
    ROGUE_TEXSTATE_DADJUST_MAX_UINT      =   8191,
    ROGUE_TEXSTATE_DADJUST_INTEGER_BITS  =      5,
    ROGUE_TEXSTATE_DADJUST_FRACTIONAL_BITS =      8,
};

static __constant const char *
ROGUE_TEXSTATE_DADJUST_to_str(const enum ROGUE_TEXSTATE_DADJUST value)
{
    switch (value) {
    case ROGUE_TEXSTATE_DADJUST_MIN_UINT: return "MIN_UINT";
    case ROGUE_TEXSTATE_DADJUST_ZERO_UINT: return "ZERO_UINT";
    case ROGUE_TEXSTATE_DADJUST_MAX_UINT: return "MAX_UINT";
    case ROGUE_TEXSTATE_DADJUST_INTEGER_BITS: return "INTEGER_BITS";
    case ROGUE_TEXSTATE_DADJUST_FRACTIONAL_BITS: return "FRACTIONAL_BITS";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_FILTER {
    ROGUE_TEXSTATE_FILTER_POINT          =      0,
    ROGUE_TEXSTATE_FILTER_LINEAR         =      1,
    ROGUE_TEXSTATE_FILTER_BICUBIC        =      2,
};

static __constant const char *
ROGUE_TEXSTATE_FILTER_to_str(const enum ROGUE_TEXSTATE_FILTER value)
{
    switch (value) {
    case ROGUE_TEXSTATE_FILTER_POINT: return "POINT";
    case ROGUE_TEXSTATE_FILTER_LINEAR: return "LINEAR";
    case ROGUE_TEXSTATE_FILTER_BICUBIC: return "BICUBIC";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_FORMAT {
    ROGUE_TEXSTATE_FORMAT_U8             =      0,
    ROGUE_TEXSTATE_FORMAT_S8             =      1,
    ROGUE_TEXSTATE_FORMAT_A4R4G4B4       =      2,
    ROGUE_TEXSTATE_FORMAT_A8R3G3B2       =      3,
    ROGUE_TEXSTATE_FORMAT_A1R5G5B5       =      4,
    ROGUE_TEXSTATE_FORMAT_R5G6B5         =      5,
    ROGUE_TEXSTATE_FORMAT_R5SG5SB6       =      6,
    ROGUE_TEXSTATE_FORMAT_U8U8           =      7,
    ROGUE_TEXSTATE_FORMAT_S8S8           =      8,
    ROGUE_TEXSTATE_FORMAT_U16            =      9,
    ROGUE_TEXSTATE_FORMAT_S16            =     10,
    ROGUE_TEXSTATE_FORMAT_F16            =     11,
    ROGUE_TEXSTATE_FORMAT_U8U8U8U8       =     12,
    ROGUE_TEXSTATE_FORMAT_S8S8S8S8       =     13,
    ROGUE_TEXSTATE_FORMAT_A2R10B10G10    =     14,
    ROGUE_TEXSTATE_FORMAT_U16U16         =     15,
    ROGUE_TEXSTATE_FORMAT_S16S16         =     16,
    ROGUE_TEXSTATE_FORMAT_F16F16         =     17,
    ROGUE_TEXSTATE_FORMAT_F32            =     18,
    ROGUE_TEXSTATE_FORMAT_F32_SIGNMASK   =     19,
    ROGUE_TEXSTATE_FORMAT_X8U8S8S8       =     20,
    ROGUE_TEXSTATE_FORMAT_X8U24          =     21,
    ROGUE_TEXSTATE_FORMAT_ST8U24         =     22,
    ROGUE_TEXSTATE_FORMAT_U8X24          =     23,
    ROGUE_TEXSTATE_FORMAT_U32            =     24,
    ROGUE_TEXSTATE_FORMAT_S32            =     25,
    ROGUE_TEXSTATE_FORMAT_SE9995         =     26,
    ROGUE_TEXSTATE_FORMAT_F11F11F10      =     27,
    ROGUE_TEXSTATE_FORMAT_F16F16F16F16   =     28,
    ROGUE_TEXSTATE_FORMAT_U16U16U16U16   =     29,
    ROGUE_TEXSTATE_FORMAT_S16S16S16S16   =     30,
    ROGUE_TEXSTATE_FORMAT_F16F16F16      =     31,
    ROGUE_TEXSTATE_FORMAT_U16U16U16      =     32,
    ROGUE_TEXSTATE_FORMAT_S16S16S16      =     33,
    ROGUE_TEXSTATE_FORMAT_F32F32         =     34,
    ROGUE_TEXSTATE_FORMAT_U32U32         =     35,
    ROGUE_TEXSTATE_FORMAT_S32S32         =     36,
    ROGUE_TEXSTATE_FORMAT_X24U8F32       =     37,
    ROGUE_TEXSTATE_FORMAT_X24X8F32       =     38,
    ROGUE_TEXSTATE_FORMAT_X24G8X32       =     39,
    ROGUE_TEXSTATE_FORMAT_YUV420_2PLANE  =     54,
    ROGUE_TEXSTATE_FORMAT_YVU420_2PLANE  =     55,
    ROGUE_TEXSTATE_FORMAT_YUV420_3PLANE  =     56,
    ROGUE_TEXSTATE_FORMAT_YVU420_3PLANE  =     57,
    ROGUE_TEXSTATE_FORMAT_U8U8U8         =     58,
    ROGUE_TEXSTATE_FORMAT_S8S8S8         =     59,
    ROGUE_TEXSTATE_FORMAT_A2F10F10F10    =     60,
    ROGUE_TEXSTATE_FORMAT_F32F32F32F32   =     61,
    ROGUE_TEXSTATE_FORMAT_U32U32U32U32   =     62,
    ROGUE_TEXSTATE_FORMAT_S32S32S32S32   =     63,
    ROGUE_TEXSTATE_FORMAT_F32F32F32      =     64,
    ROGUE_TEXSTATE_FORMAT_U32U32U32      =     65,
    ROGUE_TEXSTATE_FORMAT_S32S32S32      =     66,
    ROGUE_TEXSTATE_FORMAT_A2_XRBIAS_U10U10U10 =     67,
    ROGUE_TEXSTATE_FORMAT_O8             =     81,
    ROGUE_TEXSTATE_FORMAT_O8O8           =     82,
    ROGUE_TEXSTATE_FORMAT_R5G5B5A1       =     83,
    ROGUE_TEXSTATE_FORMAT_B6G5SR5S       =     84,
    ROGUE_TEXSTATE_FORMAT_R10B10G10A2    =     85,
    ROGUE_TEXSTATE_FORMAT_X8S8S8U8       =     86,
    ROGUE_TEXSTATE_FORMAT_U24ST8         =     87,
    ROGUE_TEXSTATE_FORMAT_F10F11F11      =     88,
    ROGUE_TEXSTATE_FORMAT_VYUY           =     89,
    ROGUE_TEXSTATE_FORMAT_UYVY           =     90,
    ROGUE_TEXSTATE_FORMAT_YVYU           =     91,
    ROGUE_TEXSTATE_FORMAT_YUYV           =     92,
    ROGUE_TEXSTATE_FORMAT_F10F10F10A2    =     93,
    ROGUE_TEXSTATE_FORMAT_YUV420_2PLANE_MACRO_BLOCK =     94,
    ROGUE_TEXSTATE_FORMAT_YVU420_2PLANE_MACRO_BLOCK =     95,
    ROGUE_TEXSTATE_FORMAT_ARGBV16_XR10   =     96,
    ROGUE_TEXSTATE_FORMAT_YVU8_422_2PLANE_PACK8 =     97,
    ROGUE_TEXSTATE_FORMAT_YVU8_444_2PLANE_PACK8 =     98,
    ROGUE_TEXSTATE_FORMAT_YVU10_444_1PLANE_PACK10 =     99,
    ROGUE_TEXSTATE_FORMAT_YVU10_422_2PLANE_PACK16 =    100,
    ROGUE_TEXSTATE_FORMAT_YVU10_420_2PLANE_PACK16 =    101,
    ROGUE_TEXSTATE_FORMAT_YVU10_444_2PLANE_PACK16 =    102,
    ROGUE_TEXSTATE_FORMAT_YUV8_422_2PLANE_PACK8 =    103,
    ROGUE_TEXSTATE_FORMAT_YUV8_444_3PLANE_PACK8 =    104,
    ROGUE_TEXSTATE_FORMAT_YUV10_444_3PLANE_PACK16 =    105,
    ROGUE_TEXSTATE_FORMAT_YVU10_420_2PLANE_PACK10 =    106,
    ROGUE_TEXSTATE_FORMAT_YUV10_420_2PLANE_PACK10 =    107,
    ROGUE_TEXSTATE_FORMAT_YVU10_422_2PLANE_PACK10 =    108,
    ROGUE_TEXSTATE_FORMAT_YUV10_422_2PLANE_PACK10 =    109,
    ROGUE_TEXSTATE_FORMAT_YUV10_444_3PLANE_PACK10 =    110,
    ROGUE_TEXSTATE_FORMAT_R8G8_B8G8      =    111,
    ROGUE_TEXSTATE_FORMAT_G8R8_G8B8      =    112,
    ROGUE_TEXSTATE_FORMAT_YVYU_IMPLIED_CSC =    113,
    ROGUE_TEXSTATE_FORMAT_VYUY_IMPLIED_CSC =    114,
    ROGUE_TEXSTATE_FORMAT_YUV10_420_2PLANE_PACK10_R =    115,
    ROGUE_TEXSTATE_FORMAT_YVU10_420_2PLANE_PACK10_R =    116,
    ROGUE_TEXSTATE_FORMAT_YUV10_420_2PLANE_PACK10_T =    117,
    ROGUE_TEXSTATE_FORMAT_YVU10_420_2PLANE_PACK10_T =    118,
    ROGUE_TEXSTATE_FORMAT_YUV8_420_2PLANE_PACK8_P =    119,
    ROGUE_TEXSTATE_FORMAT_YVU8_420_2PLANE_PACK8_P =    120,
    ROGUE_TEXSTATE_FORMAT_YUV8_420_2PLANE_PACK8_F =    121,
    ROGUE_TEXSTATE_FORMAT_YVU8_420_2PLANE_PACK8_F =    122,
    ROGUE_TEXSTATE_FORMAT_INVALID        = 4294967295,
};

static __constant const char *
ROGUE_TEXSTATE_FORMAT_to_str(const enum ROGUE_TEXSTATE_FORMAT value)
{
    switch (value) {
    case ROGUE_TEXSTATE_FORMAT_U8: return "U8";
    case ROGUE_TEXSTATE_FORMAT_S8: return "S8";
    case ROGUE_TEXSTATE_FORMAT_A4R4G4B4: return "A4R4G4B4";
    case ROGUE_TEXSTATE_FORMAT_A8R3G3B2: return "A8R3G3B2";
    case ROGUE_TEXSTATE_FORMAT_A1R5G5B5: return "A1R5G5B5";
    case ROGUE_TEXSTATE_FORMAT_R5G6B5: return "R5G6B5";
    case ROGUE_TEXSTATE_FORMAT_R5SG5SB6: return "R5sG5sB6";
    case ROGUE_TEXSTATE_FORMAT_U8U8: return "U8U8";
    case ROGUE_TEXSTATE_FORMAT_S8S8: return "S8S8";
    case ROGUE_TEXSTATE_FORMAT_U16: return "U16";
    case ROGUE_TEXSTATE_FORMAT_S16: return "S16";
    case ROGUE_TEXSTATE_FORMAT_F16: return "F16";
    case ROGUE_TEXSTATE_FORMAT_U8U8U8U8: return "U8U8U8U8";
    case ROGUE_TEXSTATE_FORMAT_S8S8S8S8: return "S8S8S8S8";
    case ROGUE_TEXSTATE_FORMAT_A2R10B10G10: return "A2R10B10G10";
    case ROGUE_TEXSTATE_FORMAT_U16U16: return "U16U16";
    case ROGUE_TEXSTATE_FORMAT_S16S16: return "S16S16";
    case ROGUE_TEXSTATE_FORMAT_F16F16: return "F16F16";
    case ROGUE_TEXSTATE_FORMAT_F32: return "F32";
    case ROGUE_TEXSTATE_FORMAT_F32_SIGNMASK: return "F32_SIGNMASK";
    case ROGUE_TEXSTATE_FORMAT_X8U8S8S8: return "X8U8S8S8";
    case ROGUE_TEXSTATE_FORMAT_X8U24: return "X8U24";
    case ROGUE_TEXSTATE_FORMAT_ST8U24: return "ST8U24";
    case ROGUE_TEXSTATE_FORMAT_U8X24: return "U8X24";
    case ROGUE_TEXSTATE_FORMAT_U32: return "U32";
    case ROGUE_TEXSTATE_FORMAT_S32: return "S32";
    case ROGUE_TEXSTATE_FORMAT_SE9995: return "SE9995";
    case ROGUE_TEXSTATE_FORMAT_F11F11F10: return "F11F11F10";
    case ROGUE_TEXSTATE_FORMAT_F16F16F16F16: return "F16F16F16F16";
    case ROGUE_TEXSTATE_FORMAT_U16U16U16U16: return "U16U16U16U16";
    case ROGUE_TEXSTATE_FORMAT_S16S16S16S16: return "S16S16S16S16";
    case ROGUE_TEXSTATE_FORMAT_F16F16F16: return "F16F16F16";
    case ROGUE_TEXSTATE_FORMAT_U16U16U16: return "U16U16U16";
    case ROGUE_TEXSTATE_FORMAT_S16S16S16: return "S16S16S16";
    case ROGUE_TEXSTATE_FORMAT_F32F32: return "F32F32";
    case ROGUE_TEXSTATE_FORMAT_U32U32: return "U32U32";
    case ROGUE_TEXSTATE_FORMAT_S32S32: return "S32S32";
    case ROGUE_TEXSTATE_FORMAT_X24U8F32: return "X24U8F32";
    case ROGUE_TEXSTATE_FORMAT_X24X8F32: return "X24X8F32";
    case ROGUE_TEXSTATE_FORMAT_X24G8X32: return "X24G8X32";
    case ROGUE_TEXSTATE_FORMAT_YUV420_2PLANE: return "YUV420_2PLANE";
    case ROGUE_TEXSTATE_FORMAT_YVU420_2PLANE: return "YVU420_2PLANE";
    case ROGUE_TEXSTATE_FORMAT_YUV420_3PLANE: return "YUV420_3PLANE";
    case ROGUE_TEXSTATE_FORMAT_YVU420_3PLANE: return "YVU420_3PLANE";
    case ROGUE_TEXSTATE_FORMAT_U8U8U8: return "U8U8U8";
    case ROGUE_TEXSTATE_FORMAT_S8S8S8: return "S8S8S8";
    case ROGUE_TEXSTATE_FORMAT_A2F10F10F10: return "A2F10F10F10";
    case ROGUE_TEXSTATE_FORMAT_F32F32F32F32: return "F32F32F32F32";
    case ROGUE_TEXSTATE_FORMAT_U32U32U32U32: return "U32U32U32U32";
    case ROGUE_TEXSTATE_FORMAT_S32S32S32S32: return "S32S32S32S32";
    case ROGUE_TEXSTATE_FORMAT_F32F32F32: return "F32F32F32";
    case ROGUE_TEXSTATE_FORMAT_U32U32U32: return "U32U32U32";
    case ROGUE_TEXSTATE_FORMAT_S32S32S32: return "S32S32S32";
    case ROGUE_TEXSTATE_FORMAT_A2_XRBIAS_U10U10U10: return "A2_XRBIAS_U10U10U10";
    case ROGUE_TEXSTATE_FORMAT_O8: return "O8";
    case ROGUE_TEXSTATE_FORMAT_O8O8: return "O8O8";
    case ROGUE_TEXSTATE_FORMAT_R5G5B5A1: return "R5G5B5A1";
    case ROGUE_TEXSTATE_FORMAT_B6G5SR5S: return "B6G5sR5s";
    case ROGUE_TEXSTATE_FORMAT_R10B10G10A2: return "R10B10G10A2";
    case ROGUE_TEXSTATE_FORMAT_X8S8S8U8: return "X8S8S8U8";
    case ROGUE_TEXSTATE_FORMAT_U24ST8: return "U24ST8";
    case ROGUE_TEXSTATE_FORMAT_F10F11F11: return "F10F11F11";
    case ROGUE_TEXSTATE_FORMAT_VYUY: return "VYUY";
    case ROGUE_TEXSTATE_FORMAT_UYVY: return "UYVY";
    case ROGUE_TEXSTATE_FORMAT_YVYU: return "YVYU";
    case ROGUE_TEXSTATE_FORMAT_YUYV: return "YUYV";
    case ROGUE_TEXSTATE_FORMAT_F10F10F10A2: return "F10F10F10A2";
    case ROGUE_TEXSTATE_FORMAT_YUV420_2PLANE_MACRO_BLOCK: return "YUV420_2PLANE_MACRO_BLOCK";
    case ROGUE_TEXSTATE_FORMAT_YVU420_2PLANE_MACRO_BLOCK: return "YVU420_2PLANE_MACRO_BLOCK";
    case ROGUE_TEXSTATE_FORMAT_ARGBV16_XR10: return "ARGBV16_XR10";
    case ROGUE_TEXSTATE_FORMAT_YVU8_422_2PLANE_PACK8: return "YVU8_422_2PLANE_PACK8";
    case ROGUE_TEXSTATE_FORMAT_YVU8_444_2PLANE_PACK8: return "YVU8_444_2PLANE_PACK8";
    case ROGUE_TEXSTATE_FORMAT_YVU10_444_1PLANE_PACK10: return "YVU10_444_1PLANE_PACK10";
    case ROGUE_TEXSTATE_FORMAT_YVU10_422_2PLANE_PACK16: return "YVU10_422_2PLANE_PACK16";
    case ROGUE_TEXSTATE_FORMAT_YVU10_420_2PLANE_PACK16: return "YVU10_420_2PLANE_PACK16";
    case ROGUE_TEXSTATE_FORMAT_YVU10_444_2PLANE_PACK16: return "YVU10_444_2PLANE_PACK16";
    case ROGUE_TEXSTATE_FORMAT_YUV8_422_2PLANE_PACK8: return "YUV8_422_2PLANE_PACK8";
    case ROGUE_TEXSTATE_FORMAT_YUV8_444_3PLANE_PACK8: return "YUV8_444_3PLANE_PACK8";
    case ROGUE_TEXSTATE_FORMAT_YUV10_444_3PLANE_PACK16: return "YUV10_444_3PLANE_PACK16";
    case ROGUE_TEXSTATE_FORMAT_YVU10_420_2PLANE_PACK10: return "YVU10_420_2PLANE_PACK10";
    case ROGUE_TEXSTATE_FORMAT_YUV10_420_2PLANE_PACK10: return "YUV10_420_2PLANE_PACK10";
    case ROGUE_TEXSTATE_FORMAT_YVU10_422_2PLANE_PACK10: return "YVU10_422_2PLANE_PACK10";
    case ROGUE_TEXSTATE_FORMAT_YUV10_422_2PLANE_PACK10: return "YUV10_422_2PLANE_PACK10";
    case ROGUE_TEXSTATE_FORMAT_YUV10_444_3PLANE_PACK10: return "YUV10_444_3PLANE_PACK10";
    case ROGUE_TEXSTATE_FORMAT_R8G8_B8G8: return "R8G8_B8G8";
    case ROGUE_TEXSTATE_FORMAT_G8R8_G8B8: return "G8R8_G8B8";
    case ROGUE_TEXSTATE_FORMAT_YVYU_IMPLIED_CSC: return "YVYU_IMPLIED_CSC";
    case ROGUE_TEXSTATE_FORMAT_VYUY_IMPLIED_CSC: return "VYUY_IMPLIED_CSC";
    case ROGUE_TEXSTATE_FORMAT_YUV10_420_2PLANE_PACK10_R: return "YUV10_420_2PLANE_PACK10_R";
    case ROGUE_TEXSTATE_FORMAT_YVU10_420_2PLANE_PACK10_R: return "YVU10_420_2PLANE_PACK10_R";
    case ROGUE_TEXSTATE_FORMAT_YUV10_420_2PLANE_PACK10_T: return "YUV10_420_2PLANE_PACK10_T";
    case ROGUE_TEXSTATE_FORMAT_YVU10_420_2PLANE_PACK10_T: return "YVU10_420_2PLANE_PACK10_T";
    case ROGUE_TEXSTATE_FORMAT_YUV8_420_2PLANE_PACK8_P: return "YUV8_420_2PLANE_PACK8_P";
    case ROGUE_TEXSTATE_FORMAT_YVU8_420_2PLANE_PACK8_P: return "YVU8_420_2PLANE_PACK8_P";
    case ROGUE_TEXSTATE_FORMAT_YUV8_420_2PLANE_PACK8_F: return "YUV8_420_2PLANE_PACK8_F";
    case ROGUE_TEXSTATE_FORMAT_YVU8_420_2PLANE_PACK8_F: return "YVU8_420_2PLANE_PACK8_F";
    case ROGUE_TEXSTATE_FORMAT_INVALID: return "INVALID";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_FORMAT_COMPRESSED {
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_4X4 =      0,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_5X4 =      1,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_5X5 =      2,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_6X5 =      3,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_6X6 =      4,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_8X5 =      5,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_8X6 =      6,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_8X8 =      7,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_10X5 =      8,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_10X6 =      9,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_10X8 =     10,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_10X10 =     11,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_12X10 =     12,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_12X12 =     13,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_PVRT2BPP =     40,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_PVRT4BPP =     41,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_PVRTII2BPP =     42,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_PVRTII4BPP =     43,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC1 =     44,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC2 =     45,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC3 =     46,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC4 =     47,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_SBC4 =     48,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC5 =     49,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_SBC5 =     50,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC6 =     51,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_SBC6 =     52,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC7 =     53,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_RGB =     68,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2A_RGBA =     69,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_PUNCHTHROUGHA =     70,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_EAC_R11_UNSIGNED =     71,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_EAC_R11_SIGNED =     72,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_EAC_RG11_UNSIGNED =     73,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_EAC_RG11_SIGNED =     74,
    ROGUE_TEXSTATE_FORMAT_COMPRESSED_INVALID = 4294967295,
};

static __constant const char *
ROGUE_TEXSTATE_FORMAT_COMPRESSED_to_str(const enum ROGUE_TEXSTATE_FORMAT_COMPRESSED value)
{
    switch (value) {
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_4X4: return "ASTC_4x4";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_5X4: return "ASTC_5x4";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_5X5: return "ASTC_5x5";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_6X5: return "ASTC_6x5";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_6X6: return "ASTC_6x6";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_8X5: return "ASTC_8x5";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_8X6: return "ASTC_8x6";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_8X8: return "ASTC_8x8";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_10X5: return "ASTC_10x5";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_10X6: return "ASTC_10x6";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_10X8: return "ASTC_10x8";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_10X10: return "ASTC_10x10";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_12X10: return "ASTC_12x10";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ASTC_12X12: return "ASTC_12x12";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_PVRT2BPP: return "PVRT2BPP";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_PVRT4BPP: return "PVRT4BPP";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_PVRTII2BPP: return "PVRTII2BPP";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_PVRTII4BPP: return "PVRTII4BPP";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC1: return "UBC1";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC2: return "UBC2";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC3: return "UBC3";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC4: return "UBC4";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_SBC4: return "SBC4";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC5: return "UBC5";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_SBC5: return "SBC5";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC6: return "UBC6";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_SBC6: return "SBC6";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_UBC7: return "UBC7";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_RGB: return "ETC2_RGB";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2A_RGBA: return "ETC2A_RGBA";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_PUNCHTHROUGHA: return "ETC2_PUNCHTHROUGHA";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_EAC_R11_UNSIGNED: return "EAC_R11_UNSIGNED";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_EAC_R11_SIGNED: return "EAC_R11_SIGNED";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_EAC_RG11_UNSIGNED: return "EAC_RG11_UNSIGNED";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_EAC_RG11_SIGNED: return "EAC_RG11_SIGNED";
    case ROGUE_TEXSTATE_FORMAT_COMPRESSED_INVALID: return "INVALID";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_GAMMA {
    ROGUE_TEXSTATE_GAMMA_OFF             =      0,
    ROGUE_TEXSTATE_GAMMA_ON              =      1,
};

static __constant const char *
ROGUE_TEXSTATE_GAMMA_to_str(const enum ROGUE_TEXSTATE_GAMMA value)
{
    switch (value) {
    case ROGUE_TEXSTATE_GAMMA_OFF: return "OFF";
    case ROGUE_TEXSTATE_GAMMA_ON: return "ON";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_SWIZ {
    ROGUE_TEXSTATE_SWIZ_SRCCHAN_0        =      0,
    ROGUE_TEXSTATE_SWIZ_SRCCHAN_1        =      1,
    ROGUE_TEXSTATE_SWIZ_SRCCHAN_2        =      2,
    ROGUE_TEXSTATE_SWIZ_SRCCHAN_3        =      3,
    ROGUE_TEXSTATE_SWIZ_SRC_ONE          =      4,
    ROGUE_TEXSTATE_SWIZ_SRC_ZERO         =      5,
};

static __constant const char *
ROGUE_TEXSTATE_SWIZ_to_str(const enum ROGUE_TEXSTATE_SWIZ value)
{
    switch (value) {
    case ROGUE_TEXSTATE_SWIZ_SRCCHAN_0: return "SRCCHAN_0";
    case ROGUE_TEXSTATE_SWIZ_SRCCHAN_1: return "SRCCHAN_1";
    case ROGUE_TEXSTATE_SWIZ_SRCCHAN_2: return "SRCCHAN_2";
    case ROGUE_TEXSTATE_SWIZ_SRCCHAN_3: return "SRCCHAN_3";
    case ROGUE_TEXSTATE_SWIZ_SRC_ONE: return "SRC_ONE";
    case ROGUE_TEXSTATE_SWIZ_SRC_ZERO: return "SRC_ZERO";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_TEXTYPE {
    ROGUE_TEXSTATE_TEXTYPE_1D            =      0,
    ROGUE_TEXSTATE_TEXTYPE_2D            =      1,
    ROGUE_TEXSTATE_TEXTYPE_3D            =      2,
    ROGUE_TEXSTATE_TEXTYPE_CUBE          =      3,
    ROGUE_TEXSTATE_TEXTYPE_STRIDE        =      4,
    ROGUE_TEXSTATE_TEXTYPE_PAGETILE      =      5,
    ROGUE_TEXSTATE_TEXTYPE_BUFFER_LOOKUP =      6,
};

static __constant const char *
ROGUE_TEXSTATE_TEXTYPE_to_str(const enum ROGUE_TEXSTATE_TEXTYPE value)
{
    switch (value) {
    case ROGUE_TEXSTATE_TEXTYPE_1D: return "_1D";
    case ROGUE_TEXSTATE_TEXTYPE_2D: return "_2D";
    case ROGUE_TEXSTATE_TEXTYPE_3D: return "_3D";
    case ROGUE_TEXSTATE_TEXTYPE_CUBE: return "CUBE";
    case ROGUE_TEXSTATE_TEXTYPE_STRIDE: return "STRIDE";
    case ROGUE_TEXSTATE_TEXTYPE_PAGETILE: return "PAGETILE";
    case ROGUE_TEXSTATE_TEXTYPE_BUFFER_LOOKUP: return "BUFFER_LOOKUP";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_TWOCOMP_GAMMA {
    ROGUE_TEXSTATE_TWOCOMP_GAMMA_OFF     =      0,
    ROGUE_TEXSTATE_TWOCOMP_GAMMA_R       =      1,
    ROGUE_TEXSTATE_TWOCOMP_GAMMA_RG      =      3,
};

static __constant const char *
ROGUE_TEXSTATE_TWOCOMP_GAMMA_to_str(const enum ROGUE_TEXSTATE_TWOCOMP_GAMMA value)
{
    switch (value) {
    case ROGUE_TEXSTATE_TWOCOMP_GAMMA_OFF: return "OFF";
    case ROGUE_TEXSTATE_TWOCOMP_GAMMA_R: return "R";
    case ROGUE_TEXSTATE_TWOCOMP_GAMMA_RG: return "RG";
    default: return NULL;
    }
}

enum ROGUE_TEXSTATE_CHROMAINTERP {
    ROGUE_TEXSTATE_CHROMAINTERP_ZERO     =      0,
    ROGUE_TEXSTATE_CHROMAINTERP_ENABLED  =      1,
};

static __constant const char *
ROGUE_TEXSTATE_CHROMAINTERP_to_str(const enum ROGUE_TEXSTATE_CHROMAINTERP value)
{
    switch (value) {
    case ROGUE_TEXSTATE_CHROMAINTERP_ZERO: return "ZERO";
    case ROGUE_TEXSTATE_CHROMAINTERP_ENABLED: return "ENABLED";
    default: return NULL;
    }
}

#define ROGUE_TEXSTATE_IMAGE_WORD0_length      2
#define ROGUE_TEXSTATE_IMAGE_WORD0_header       \


/* Helper macros for width */
#define ROGUE_TEXSTATE_IMAGE_WORD0_WIDTH_MAX_SIZE 16383

/* Helper macros for texformat */
#define ROGUE_TEXSTATE_IMAGE_WORD0_TEXFORMAT_MAX_SIZE 127

struct ROGUE_TEXSTATE_IMAGE_WORD0 {
    uint32_t                             smpcnt;
    uint32_t                             height;
    uint32_t                             width;
    enum ROGUE_TEXSTATE_FORMAT           texformat;
    enum ROGUE_TEXSTATE_FORMAT_COMPRESSED texformat_compressed;
    uint32_t                             minlod;
    enum ROGUE_TEXSTATE_SWIZ             swiz0;
    enum ROGUE_TEXSTATE_SWIZ             swiz1;
    enum ROGUE_TEXSTATE_SWIZ             swiz2;
    enum ROGUE_TEXSTATE_SWIZ             swiz3;
    enum ROGUE_TEXSTATE_TWOCOMP_GAMMA    twocomp_gamma;
    enum ROGUE_TEXSTATE_GAMMA            gamma;
    enum ROGUE_TEXSTATE_TEXTYPE          textype;
};

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_IMAGE_WORD0_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_TEXSTATE_IMAGE_WORD0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->texformat, 27, 33) |
      __pvr_uint(values->smpcnt, 62, 63) |
      __pvr_uint(values->height, 48, 61) |
      __pvr_uint(values->width, 34, 47) |
      __pvr_uint(values->texformat_compressed, 27, 33) |
      __pvr_uint(values->minlod, 17, 26) |
      __pvr_uint(values->swiz0, 14, 16) |
      __pvr_uint(values->swiz1, 11, 13) |
      __pvr_uint(values->swiz2, 8, 10) |
      __pvr_uint(values->swiz3, 5, 7) |
      __pvr_uint(values->twocomp_gamma, 3, 4) |
      __pvr_uint(values->gamma, 3, 3) |
      __pvr_uint(values->textype, 0, 2);
    dw[0] = v0;
    dw[1] = v0 >> 32;
}

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_IMAGE_WORD0_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_TEXSTATE_IMAGE_WORD0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->texformat = __pvr_uint_unpack(v0, 27, 33);
    values->smpcnt = __pvr_uint_unpack(v0, 62, 63);
    values->height = __pvr_uint_unpack(v0, 48, 61);
    values->width = __pvr_uint_unpack(v0, 34, 47);
    values->texformat_compressed = __pvr_uint_unpack(v0, 27, 33);
    values->minlod = __pvr_uint_unpack(v0, 17, 26);
    values->swiz0 = __pvr_uint_unpack(v0, 14, 16);
    values->swiz1 = __pvr_uint_unpack(v0, 11, 13);
    values->swiz2 = __pvr_uint_unpack(v0, 8, 10);
    values->swiz3 = __pvr_uint_unpack(v0, 5, 7);
    values->twocomp_gamma = __pvr_uint_unpack(v0, 3, 4);
    values->gamma = __pvr_uint_unpack(v0, 3, 3);
    values->textype = __pvr_uint_unpack(v0, 0, 2);
}

#define ROGUE_TEXSTATE_IMAGE_WORD1_length      2
#define ROGUE_TEXSTATE_IMAGE_WORD1_header       \


/* Helper macros for texaddr */
#define ROGUE_TEXSTATE_IMAGE_WORD1_TEXADDR_ALIGNMENT 4

/* Helper macros for depth */
#define ROGUE_TEXSTATE_IMAGE_WORD1_DEPTH_MAX_SIZE 2047

struct ROGUE_TEXSTATE_IMAGE_WORD1 {
    uint32_t                             baselevel;
    bool                                 alpha_msb;
    bool                                 border;
/* if TPU_IMAGE_STATE_V2 is supported use: */
/* if TFBC is supported use: */
    enum ROGUE_TEXSTATE_COMPRESSION_LEVEL lossy_compression_mode;
/* endif TFBC */
    enum ROGUE_TEXSTATE_COMPRESSION_MODE tpu_image_state_v2_compression_mode;
/* else TPU_IMAGE_STATE_V2 is not-supported use: */
    bool                                 tile_size;
    bool                                 index_lookup;
    bool                                 frame_buffer_compression;
    bool                                 frame_buffer_compression_addressing_mode;
/* endif TPU_IMAGE_STATE_V2 */
    __pvr_address_type                   texaddr;
    bool                                 mipmaps_present;
    uint32_t                             depth;
    uint32_t                             num_mip_levels;
};

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_IMAGE_WORD1_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_TEXSTATE_IMAGE_WORD1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->baselevel, 60, 63) |
      __pvr_uint(values->alpha_msb, 59, 59) |
      __pvr_uint(values->border, 58, 58) |
      __pvr_uint(values->lossy_compression_mode, 56, 57) |
      __pvr_uint(values->tpu_image_state_v2_compression_mode, 54, 56) |
      __pvr_uint(values->tile_size, 57, 57) |
      __pvr_uint(values->index_lookup, 56, 56) |
      __pvr_uint(values->frame_buffer_compression, 55, 55) |
      __pvr_uint(values->frame_buffer_compression_addressing_mode, 54, 54) |
      __pvr_uint(values->mipmaps_present, 15, 15) |
      __pvr_uint(values->depth, 4, 14) |
      __pvr_uint(values->num_mip_levels, 0, 3);
    const uint64_t v0_address =
      __pvr_address(values->texaddr, 2, 16, 53);
    dw[0] = v0_address | v0;
    dw[1] = ((v0_address) >> 32) | (v0 >> 32);
}

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_IMAGE_WORD1_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_TEXSTATE_IMAGE_WORD1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->texaddr = __pvr_address_unpack(v0, 2, 16, 53);
    values->baselevel = __pvr_uint_unpack(v0, 60, 63);
    values->alpha_msb = __pvr_uint_unpack(v0, 59, 59);
    values->border = __pvr_uint_unpack(v0, 58, 58);
    values->lossy_compression_mode = __pvr_uint_unpack(v0, 56, 57);
    values->tpu_image_state_v2_compression_mode = __pvr_uint_unpack(v0, 54, 56);
    values->tile_size = __pvr_uint_unpack(v0, 57, 57);
    values->index_lookup = __pvr_uint_unpack(v0, 56, 56);
    values->frame_buffer_compression = __pvr_uint_unpack(v0, 55, 55);
    values->frame_buffer_compression_addressing_mode = __pvr_uint_unpack(v0, 54, 54);
    values->mipmaps_present = __pvr_uint_unpack(v0, 15, 15);
    values->depth = __pvr_uint_unpack(v0, 4, 14);
    values->num_mip_levels = __pvr_uint_unpack(v0, 0, 3);
}

#define ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1_length      2
#define ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1_header\


/* Helper macros for texaddr */
#define ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1_TEXADDR_ALIGNMENT 4

struct ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1 {
    uint32_t                             num_mip_levels;
    bool                                 alpha_msb;
/* if TPU_IMAGE_STATE_V2 is supported use: */
/* if TFBC is supported use: */
    enum ROGUE_TEXSTATE_COMPRESSION_LEVEL lossy_compression_mode;
/* endif TFBC */
    enum ROGUE_TEXSTATE_COMPRESSION_MODE tpu_image_state_v2_compression_mode;
/* else TPU_IMAGE_STATE_V2 is not-supported use: */
    bool                                 tile_size;
    bool                                 index_lookup;
    bool                                 frame_buffer_compression;
    bool                                 frame_buffer_compression_addressing_mode;
/* endif TPU_IMAGE_STATE_V2 */
    __pvr_address_type                   texaddr;
    bool                                 mipmaps_present;
    uint32_t                             stride;
};

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1_pack(__attribute__((unused)) void * restrict dst,
                                       __attribute__((unused)) const struct ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->num_mip_levels, 60, 63) |
      __pvr_uint(values->alpha_msb, 59, 59) |
      __pvr_uint(values->lossy_compression_mode, 56, 57) |
      __pvr_uint(values->tpu_image_state_v2_compression_mode, 54, 56) |
      __pvr_uint(values->tile_size, 57, 57) |
      __pvr_uint(values->index_lookup, 56, 56) |
      __pvr_uint(values->frame_buffer_compression, 55, 55) |
      __pvr_uint(values->frame_buffer_compression_addressing_mode, 54, 54) |
      __pvr_uint(values->mipmaps_present, 15, 15) |
      __pvr_uint(values->stride, 0, 14);
    const uint64_t v0_address =
      __pvr_address(values->texaddr, 2, 16, 53);
    dw[0] = v0_address | v0;
    dw[1] = ((v0_address) >> 32) | (v0 >> 32);
}

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1_unpack(__attribute__((unused)) const void * restrict src,
                                         __attribute__((unused)) struct ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->texaddr = __pvr_address_unpack(v0, 2, 16, 53);
    values->num_mip_levels = __pvr_uint_unpack(v0, 60, 63);
    values->alpha_msb = __pvr_uint_unpack(v0, 59, 59);
    values->lossy_compression_mode = __pvr_uint_unpack(v0, 56, 57);
    values->tpu_image_state_v2_compression_mode = __pvr_uint_unpack(v0, 54, 56);
    values->tile_size = __pvr_uint_unpack(v0, 57, 57);
    values->index_lookup = __pvr_uint_unpack(v0, 56, 56);
    values->frame_buffer_compression = __pvr_uint_unpack(v0, 55, 55);
    values->frame_buffer_compression_addressing_mode = __pvr_uint_unpack(v0, 54, 54);
    values->mipmaps_present = __pvr_uint_unpack(v0, 15, 15);
    values->stride = __pvr_uint_unpack(v0, 0, 14);
}

#define ROGUE_TEXSTATE_YUV_IMAGE_WORD1_length      2
#define ROGUE_TEXSTATE_YUV_IMAGE_WORD1_header   \


/* Helper macros for texaddr */
#define ROGUE_TEXSTATE_YUV_IMAGE_WORD1_TEXADDR_ALIGNMENT 4

struct ROGUE_TEXSTATE_YUV_IMAGE_WORD1 {
    uint32_t                             csc_coeff_index;
    enum ROGUE_TEXSTATE_CHROMAINTERP     chroma_interpolation_v;
    enum ROGUE_TEXSTATE_CHROMAINTERP     chroma_interpolation_u;
    uint32_t                             stride;
    __pvr_address_type                   texaddr;
};

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_YUV_IMAGE_WORD1_pack(__attribute__((unused)) void * restrict dst,
                                    __attribute__((unused)) const struct ROGUE_TEXSTATE_YUV_IMAGE_WORD1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->csc_coeff_index, 60, 63) |
      __pvr_uint(values->chroma_interpolation_v, 59, 59) |
      __pvr_uint(values->chroma_interpolation_u, 53, 53) |
      __pvr_uint(values->stride, 38, 52);
    const uint64_t v0_address =
      __pvr_address(values->texaddr, 2, 0, 37);
    dw[0] = v0_address | v0;
    dw[1] = ((v0_address) >> 32) | (v0 >> 32);
}

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_YUV_IMAGE_WORD1_unpack(__attribute__((unused)) const void * restrict src,
                                      __attribute__((unused)) struct ROGUE_TEXSTATE_YUV_IMAGE_WORD1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->texaddr = __pvr_address_unpack(v0, 2, 0, 37);
    values->csc_coeff_index = __pvr_uint_unpack(v0, 60, 63);
    values->chroma_interpolation_v = __pvr_uint_unpack(v0, 59, 59);
    values->chroma_interpolation_u = __pvr_uint_unpack(v0, 53, 53);
    values->stride = __pvr_uint_unpack(v0, 38, 52);
}

#define ROGUE_TEXSTATE_SAMPLER_WORD0_length      2
#define ROGUE_TEXSTATE_SAMPLER_WORD0_header     \


/* Helper macros for texaddr_plane2_lo */
#define ROGUE_TEXSTATE_SAMPLER_WORD0_TEXADDR_PLANE2_LO_ALIGNMENT 4

/* Helper macros for bordercolor_index */
#define ROGUE_TEXSTATE_SAMPLER_WORD0_BORDERCOLOR_INDEX_MAX_SIZE 63

struct ROGUE_TEXSTATE_SAMPLER_WORD0 {
    __pvr_address_type                   texaddr_plane2_lo;
    enum ROGUE_TEXSTATE_CMP_MODE         cmp_mode;
    enum ROGUE_TEXSTATE_ADDRMODE         addrmode_w;
    uint32_t                             bordercolor_index;
    bool                                 non_normalized_coords;
    bool                                 lumakey_alphamult;
    bool                                 lumakey;
    enum ROGUE_TEXSTATE_ADDRMODE         addrmode_v;
    enum ROGUE_TEXSTATE_ADDRMODE         addrmode_u;
    bool                                 mipfilter;
    enum ROGUE_TEXSTATE_FILTER           minfilter;
    enum ROGUE_TEXSTATE_FILTER           magfilter;
    enum ROGUE_TEXSTATE_ANISOCTL         anisoctl;
    enum ROGUE_TEXSTATE_CLAMP            maxlod;
    enum ROGUE_TEXSTATE_CLAMP            minlod;
    enum ROGUE_TEXSTATE_DADJUST          dadjust;
};

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_SAMPLER_WORD0_pack(__attribute__((unused)) void * restrict dst,
                                  __attribute__((unused)) const struct ROGUE_TEXSTATE_SAMPLER_WORD0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->maxlod, 23, 32) |
      __pvr_uint(values->cmp_mode, 59, 61) |
      __pvr_uint(values->addrmode_w, 56, 58) |
      __pvr_uint(values->bordercolor_index, 50, 55) |
      __pvr_uint(values->non_normalized_coords, 49, 49) |
      __pvr_uint(values->lumakey_alphamult, 48, 48) |
      __pvr_uint(values->lumakey, 47, 47) |
      __pvr_uint(values->addrmode_v, 44, 46) |
      __pvr_uint(values->addrmode_u, 41, 43) |
      __pvr_uint(values->mipfilter, 40, 40) |
      __pvr_uint(values->minfilter, 38, 39) |
      __pvr_uint(values->magfilter, 36, 37) |
      __pvr_uint(values->anisoctl, 33, 35) |
      __pvr_uint(values->minlod, 13, 22) |
      __pvr_uint(values->dadjust, 0, 12);
    const uint64_t v0_address =
      __pvr_address(values->texaddr_plane2_lo, 2, 50, 63);
    dw[0] = v0_address | v0;
    dw[1] = ((v0_address) >> 32) | (v0 >> 32);
}

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_SAMPLER_WORD0_unpack(__attribute__((unused)) const void * restrict src,
                                    __attribute__((unused)) struct ROGUE_TEXSTATE_SAMPLER_WORD0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->maxlod = __pvr_uint_unpack(v0, 23, 32);
    values->texaddr_plane2_lo = __pvr_address_unpack(v0, 2, 50, 63);
    values->cmp_mode = __pvr_uint_unpack(v0, 59, 61);
    values->addrmode_w = __pvr_uint_unpack(v0, 56, 58);
    values->bordercolor_index = __pvr_uint_unpack(v0, 50, 55);
    values->non_normalized_coords = __pvr_uint_unpack(v0, 49, 49);
    values->lumakey_alphamult = __pvr_uint_unpack(v0, 48, 48);
    values->lumakey = __pvr_uint_unpack(v0, 47, 47);
    values->addrmode_v = __pvr_uint_unpack(v0, 44, 46);
    values->addrmode_u = __pvr_uint_unpack(v0, 41, 43);
    values->mipfilter = __pvr_uint_unpack(v0, 40, 40);
    values->minfilter = __pvr_uint_unpack(v0, 38, 39);
    values->magfilter = __pvr_uint_unpack(v0, 36, 37);
    values->anisoctl = __pvr_uint_unpack(v0, 33, 35);
    values->minlod = __pvr_uint_unpack(v0, 13, 22);
    values->dadjust = __pvr_uint_unpack(v0, 0, 12);
}

#define ROGUE_TEXSTATE_SAMPLER_WORD1_length      2
#define ROGUE_TEXSTATE_SAMPLER_WORD1_header     \


/* Helper macros for texaddr_plane3 */
#define ROGUE_TEXSTATE_SAMPLER_WORD1_TEXADDR_PLANE3_ALIGNMENT 4

/* Helper macros for texaddr_plane2_hi */
#define ROGUE_TEXSTATE_SAMPLER_WORD1_TEXADDR_PLANE2_HI_ALIGNMENT 65536

struct ROGUE_TEXSTATE_SAMPLER_WORD1 {
    bool                                 cemedge_dontfilter;
    __pvr_address_type                   texaddr_plane3;
    __pvr_address_type                   texaddr_plane2_hi;
};

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_SAMPLER_WORD1_pack(__attribute__((unused)) void * restrict dst,
                                  __attribute__((unused)) const struct ROGUE_TEXSTATE_SAMPLER_WORD1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint64_t v0 =
      __pvr_uint(values->cemedge_dontfilter, 63, 63);
    const uint64_t v0_address =
      __pvr_address(values->texaddr_plane3, 2, 24, 61);
    const uint64_t v1_address =
      __pvr_address(values->texaddr_plane2_hi, 16, 0, 23);
    dw[0] = v0_address |
            v1_address | v0;
    dw[1] = ((v0_address |
            v1_address) >> 32) | (v0 >> 32);
}

static inline __attribute__((always_inline)) void
ROGUE_TEXSTATE_SAMPLER_WORD1_unpack(__attribute__((unused)) const void * restrict src,
                                    __attribute__((unused)) struct ROGUE_TEXSTATE_SAMPLER_WORD1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    const uint64_t v0 = dw[0] | ((uint64_t)dw[1] << 32);
    values->texaddr_plane3 = __pvr_address_unpack(v0, 2, 24, 61);
    values->cemedge_dontfilter = __pvr_uint_unpack(v0, 63, 63);
    values->texaddr_plane2_hi = __pvr_address_unpack(v0, 16, 0, 23);
}

#endif /* TEXSTATE_H */
