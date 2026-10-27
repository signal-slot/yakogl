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

#ifndef VDM_H
#define VDM_H

#include "csbgen/pvr_packet_helpers.h"

#ifndef __OPENCL_VERSION__
#define __constant
#endif


#define ROGUE_VDMCTRL_GUARD_SIZE_DEFAULT         64

enum ROGUE_VDMCTRL_BLOCK_TYPE {
    ROGUE_VDMCTRL_BLOCK_TYPE_PPP_STATE_UPDATE =      0,
    ROGUE_VDMCTRL_BLOCK_TYPE_PDS_STATE_UPDATE =      1,
    ROGUE_VDMCTRL_BLOCK_TYPE_VDM_STATE_UPDATE =      2,
    ROGUE_VDMCTRL_BLOCK_TYPE_INDEX_LIST  =      3,
    ROGUE_VDMCTRL_BLOCK_TYPE_STREAM_LINK =      4,
    ROGUE_VDMCTRL_BLOCK_TYPE_STREAM_RETURN =      5,
    ROGUE_VDMCTRL_BLOCK_TYPE_STREAM_TERMINATE =      6,
    ROGUE_VDMCTRL_BLOCK_TYPE_CONTROL     =      7,
};

static __constant const char *
ROGUE_VDMCTRL_BLOCK_TYPE_to_str(const enum ROGUE_VDMCTRL_BLOCK_TYPE value)
{
    switch (value) {
    case ROGUE_VDMCTRL_BLOCK_TYPE_PPP_STATE_UPDATE: return "PPP_STATE_UPDATE";
    case ROGUE_VDMCTRL_BLOCK_TYPE_PDS_STATE_UPDATE: return "PDS_STATE_UPDATE";
    case ROGUE_VDMCTRL_BLOCK_TYPE_VDM_STATE_UPDATE: return "VDM_STATE_UPDATE";
    case ROGUE_VDMCTRL_BLOCK_TYPE_INDEX_LIST: return "INDEX_LIST";
    case ROGUE_VDMCTRL_BLOCK_TYPE_STREAM_LINK: return "STREAM_LINK";
    case ROGUE_VDMCTRL_BLOCK_TYPE_STREAM_RETURN: return "STREAM_RETURN";
    case ROGUE_VDMCTRL_BLOCK_TYPE_STREAM_TERMINATE: return "STREAM_TERMINATE";
    case ROGUE_VDMCTRL_BLOCK_TYPE_CONTROL: return "CONTROL";
    default: return NULL;
    }
}

enum ROGUE_VDMCTRL_DM_TARGET {
    ROGUE_VDMCTRL_DM_TARGET_VDM          =      0,
    ROGUE_VDMCTRL_DM_TARGET_DDM          =      1,
};

static __constant const char *
ROGUE_VDMCTRL_DM_TARGET_to_str(const enum ROGUE_VDMCTRL_DM_TARGET value)
{
    switch (value) {
    case ROGUE_VDMCTRL_DM_TARGET_VDM: return "VDM";
    case ROGUE_VDMCTRL_DM_TARGET_DDM: return "DDM";
    default: return NULL;
    }
}

enum ROGUE_VDMCTRL_FLATSHADE_CONTROL {
    ROGUE_VDMCTRL_FLATSHADE_CONTROL_VERTEX_0 =      0,
    ROGUE_VDMCTRL_FLATSHADE_CONTROL_VERTEX_1 =      1,
    ROGUE_VDMCTRL_FLATSHADE_CONTROL_VERTEX_2 =      2,
};

static __constant const char *
ROGUE_VDMCTRL_FLATSHADE_CONTROL_to_str(const enum ROGUE_VDMCTRL_FLATSHADE_CONTROL value)
{
    switch (value) {
    case ROGUE_VDMCTRL_FLATSHADE_CONTROL_VERTEX_0: return "VERTEX_0";
    case ROGUE_VDMCTRL_FLATSHADE_CONTROL_VERTEX_1: return "VERTEX_1";
    case ROGUE_VDMCTRL_FLATSHADE_CONTROL_VERTEX_2: return "VERTEX_2";
    default: return NULL;
    }
}

enum ROGUE_VDMCTRL_INDEX_SIZE {
    ROGUE_VDMCTRL_INDEX_SIZE_B8          =      0,
    ROGUE_VDMCTRL_INDEX_SIZE_B16         =      1,
    ROGUE_VDMCTRL_INDEX_SIZE_B32         =      2,
};

static __constant const char *
ROGUE_VDMCTRL_INDEX_SIZE_to_str(const enum ROGUE_VDMCTRL_INDEX_SIZE value)
{
    switch (value) {
    case ROGUE_VDMCTRL_INDEX_SIZE_B8: return "B8";
    case ROGUE_VDMCTRL_INDEX_SIZE_B16: return "B16";
    case ROGUE_VDMCTRL_INDEX_SIZE_B32: return "B32";
    default: return NULL;
    }
}

enum ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY {
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_POINT_LIST =      0,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_LIST =      1,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_LIST_ADJ =      2,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_STRIP =      3,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_STRIP_ADJ =      4,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_LOOP =      5,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_LIST =      6,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_LIST_ADJ =      7,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_LIST_EDGE =      8,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_STRIP =      9,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_STRIP_ADJ =     10,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_FAN =     11,
    ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_PATCH_LIST =     12,
};

static __constant const char *
ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_to_str(const enum ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY value)
{
    switch (value) {
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_POINT_LIST: return "POINT_LIST";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_LIST: return "LINE_LIST";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_LIST_ADJ: return "LINE_LIST_ADJ";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_STRIP: return "LINE_STRIP";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_STRIP_ADJ: return "LINE_STRIP_ADJ";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_LOOP: return "LINE_LOOP";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_LIST: return "TRI_LIST";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_LIST_ADJ: return "TRI_LIST_ADJ";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_LIST_EDGE: return "TRI_LIST_EDGE";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_STRIP: return "TRI_STRIP";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_STRIP_ADJ: return "TRI_STRIP_ADJ";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_FAN: return "TRI_FAN";
    case ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_PATCH_LIST: return "PATCH_LIST";
    default: return NULL;
    }
}

enum ROGUE_VDMCTRL_SD_TYPE {
    ROGUE_VDMCTRL_SD_TYPE_NONE           =      0,
    ROGUE_VDMCTRL_SD_TYPE_PDS            =      1,
    ROGUE_VDMCTRL_SD_TYPE_USC            =      2,
};

static __constant const char *
ROGUE_VDMCTRL_SD_TYPE_to_str(const enum ROGUE_VDMCTRL_SD_TYPE value)
{
    switch (value) {
    case ROGUE_VDMCTRL_SD_TYPE_NONE: return "NONE";
    case ROGUE_VDMCTRL_SD_TYPE_PDS: return "PDS";
    case ROGUE_VDMCTRL_SD_TYPE_USC: return "USC";
    default: return NULL;
    }
}

enum ROGUE_VDMCTRL_USC_TARGET {
    ROGUE_VDMCTRL_USC_TARGET_ALL         =      0,
    ROGUE_VDMCTRL_USC_TARGET_ANY         =      1,
};

static __constant const char *
ROGUE_VDMCTRL_USC_TARGET_to_str(const enum ROGUE_VDMCTRL_USC_TARGET value)
{
    switch (value) {
    case ROGUE_VDMCTRL_USC_TARGET_ALL: return "ALL";
    case ROGUE_VDMCTRL_USC_TARGET_ANY: return "ANY";
    default: return NULL;
    }
}

enum ROGUE_VDMCTRL_UVS_SCRATCH_SIZE_SELECT {
    ROGUE_VDMCTRL_UVS_SCRATCH_SIZE_SELECT_FIVE =      0,
    ROGUE_VDMCTRL_UVS_SCRATCH_SIZE_SELECT_ONE =      1,
};

static __constant const char *
ROGUE_VDMCTRL_UVS_SCRATCH_SIZE_SELECT_to_str(const enum ROGUE_VDMCTRL_UVS_SCRATCH_SIZE_SELECT value)
{
    switch (value) {
    case ROGUE_VDMCTRL_UVS_SCRATCH_SIZE_SELECT_FIVE: return "FIVE";
    case ROGUE_VDMCTRL_UVS_SCRATCH_SIZE_SELECT_ONE: return "ONE";
    default: return NULL;
    }
}

#define ROGUE_VDMCTRL_PPP_STATE0_length        1
#define ROGUE_VDMCTRL_PPP_STATE0_header         \
    .block_type                          = ROGUE_VDMCTRL_BLOCK_TYPE_PPP_STATE_UPDATE

/* Helper macros for addrmsb */
#define ROGUE_VDMCTRL_PPP_STATE0_ADDRMSB_ALIGNMENT 4294967296

struct ROGUE_VDMCTRL_PPP_STATE0 {
    enum ROGUE_VDMCTRL_BLOCK_TYPE        block_type;
    uint32_t                             word_count;
    __pvr_address_type                   addrmsb;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PPP_STATE0_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_PPP_STATE0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint32_t v0 =
      __pvr_uint(values->block_type, 29, 31) |
      __pvr_uint(values->word_count, 8, 15);
    dw[0] = __pvr_address(values->addrmsb, 32, 0, 7) | v0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PPP_STATE0_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_PPP_STATE0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->block_type = __pvr_uint_unpack(dw[0], 29, 31);
    values->word_count = __pvr_uint_unpack(dw[0], 8, 15);
    values->addrmsb = __pvr_address_unpack(dw[0], 32, 0, 7);
}

#define ROGUE_VDMCTRL_PPP_STATE1_length        1
#define ROGUE_VDMCTRL_PPP_STATE1_header         \


/* Helper macros for addrlsb */
#define ROGUE_VDMCTRL_PPP_STATE1_ADDRLSB_ALIGNMENT 4

struct ROGUE_VDMCTRL_PPP_STATE1 {
    __pvr_address_type                   addrlsb;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PPP_STATE1_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_PPP_STATE1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->addrlsb, 2, 2, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PPP_STATE1_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_PPP_STATE1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->addrlsb = __pvr_address_unpack(dw[0], 2, 2, 31);
}

#define ROGUE_VDMCTRL_PDS_STATE0_length        1
#define ROGUE_VDMCTRL_PDS_STATE0_header         \
    .block_type                          = ROGUE_VDMCTRL_BLOCK_TYPE_PDS_STATE_UPDATE

/* Helper macros for usc_common_size */
#define ROGUE_VDMCTRL_PDS_STATE0_USC_COMMON_SIZE_UNIT_SIZE 64

/* Helper macros for usc_unified_size */
#define ROGUE_VDMCTRL_PDS_STATE0_USC_UNIFIED_SIZE_UNIT_SIZE 16

/* Helper macros for pds_temp_size */
#define ROGUE_VDMCTRL_PDS_STATE0_PDS_TEMP_SIZE_UNIT_SIZE 16

/* Helper macros for pds_data_size */
#define ROGUE_VDMCTRL_PDS_STATE0_PDS_DATA_SIZE_UNIT_SIZE 16

struct ROGUE_VDMCTRL_PDS_STATE0 {
    enum ROGUE_VDMCTRL_BLOCK_TYPE        block_type;
    enum ROGUE_VDMCTRL_DM_TARGET         dm_target;
    enum ROGUE_VDMCTRL_USC_TARGET        usc_target;
    uint32_t                             usc_common_size;
    uint32_t                             usc_unified_size;
    uint32_t                             pds_temp_size;
    uint32_t                             pds_data_size;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PDS_STATE0_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_PDS_STATE0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->block_type, 29, 31) |
      __pvr_uint(values->dm_target, 28, 28) |
      __pvr_uint(values->usc_target, 25, 25) |
      __pvr_uint(values->usc_common_size, 16, 24) |
      __pvr_uint(values->usc_unified_size, 10, 15) |
      __pvr_uint(values->pds_temp_size, 6, 9) |
      __pvr_uint(values->pds_data_size, 0, 5);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PDS_STATE0_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_PDS_STATE0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->block_type = __pvr_uint_unpack(dw[0], 29, 31);
    values->dm_target = __pvr_uint_unpack(dw[0], 28, 28);
    values->usc_target = __pvr_uint_unpack(dw[0], 25, 25);
    values->usc_common_size = __pvr_uint_unpack(dw[0], 16, 24);
    values->usc_unified_size = __pvr_uint_unpack(dw[0], 10, 15);
    values->pds_temp_size = __pvr_uint_unpack(dw[0], 6, 9);
    values->pds_data_size = __pvr_uint_unpack(dw[0], 0, 5);
}

#define ROGUE_VDMCTRL_PDS_STATE1_length        1
#define ROGUE_VDMCTRL_PDS_STATE1_header         \


/* Helper macros for pds_data_addr */
#define ROGUE_VDMCTRL_PDS_STATE1_PDS_DATA_ADDR_ALIGNMENT 16

struct ROGUE_VDMCTRL_PDS_STATE1 {
    __pvr_address_type                   pds_data_addr;
    enum ROGUE_VDMCTRL_SD_TYPE           sd_type;
    enum ROGUE_VDMCTRL_SD_TYPE           sd_next_type;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PDS_STATE1_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_PDS_STATE1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint32_t v0 =
      __pvr_uint(values->sd_type, 2, 3) |
      __pvr_uint(values->sd_next_type, 0, 1);
    dw[0] = __pvr_address(values->pds_data_addr, 4, 4, 31) | v0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PDS_STATE1_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_PDS_STATE1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_data_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
    values->sd_type = __pvr_uint_unpack(dw[0], 2, 3);
    values->sd_next_type = __pvr_uint_unpack(dw[0], 0, 1);
}

#define ROGUE_VDMCTRL_PDS_STATE2_length        1
#define ROGUE_VDMCTRL_PDS_STATE2_header         \


/* Helper macros for pds_code_addr */
#define ROGUE_VDMCTRL_PDS_STATE2_PDS_CODE_ADDR_ALIGNMENT 16

struct ROGUE_VDMCTRL_PDS_STATE2 {
    __pvr_address_type                   pds_code_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PDS_STATE2_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_PDS_STATE2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->pds_code_addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_PDS_STATE2_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_PDS_STATE2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_code_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_VDMCTRL_STREAM_LINK0_length      1
#define ROGUE_VDMCTRL_STREAM_LINK0_header       \
    .block_type                          = ROGUE_VDMCTRL_BLOCK_TYPE_STREAM_LINK

/* Helper macros for link_addrmsb */
#define ROGUE_VDMCTRL_STREAM_LINK0_LINK_ADDRMSB_ALIGNMENT 4294967296

struct ROGUE_VDMCTRL_STREAM_LINK0 {
    enum ROGUE_VDMCTRL_BLOCK_TYPE        block_type;
    bool                                 with_return;
    bool                                 compare_present;
    uint32_t                             compare_mode;
    uint32_t                             compare_data;
    __pvr_address_type                   link_addrmsb;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_STREAM_LINK0_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_VDMCTRL_STREAM_LINK0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint32_t v0 =
      __pvr_uint(values->block_type, 29, 31) |
      __pvr_uint(values->with_return, 28, 28) |
      __pvr_uint(values->compare_present, 27, 27) |
      __pvr_uint(values->compare_mode, 24, 26) |
      __pvr_uint(values->compare_data, 8, 23);
    dw[0] = __pvr_address(values->link_addrmsb, 32, 0, 7) | v0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_STREAM_LINK0_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_VDMCTRL_STREAM_LINK0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->block_type = __pvr_uint_unpack(dw[0], 29, 31);
    values->with_return = __pvr_uint_unpack(dw[0], 28, 28);
    values->compare_present = __pvr_uint_unpack(dw[0], 27, 27);
    values->compare_mode = __pvr_uint_unpack(dw[0], 24, 26);
    values->compare_data = __pvr_uint_unpack(dw[0], 8, 23);
    values->link_addrmsb = __pvr_address_unpack(dw[0], 32, 0, 7);
}

#define ROGUE_VDMCTRL_STREAM_LINK1_length      1
#define ROGUE_VDMCTRL_STREAM_LINK1_header       \


/* Helper macros for link_addrlsb */
#define ROGUE_VDMCTRL_STREAM_LINK1_LINK_ADDRLSB_ALIGNMENT 4

struct ROGUE_VDMCTRL_STREAM_LINK1 {
    __pvr_address_type                   link_addrlsb;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_STREAM_LINK1_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_VDMCTRL_STREAM_LINK1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->link_addrlsb, 2, 2, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_STREAM_LINK1_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_VDMCTRL_STREAM_LINK1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->link_addrlsb = __pvr_address_unpack(dw[0], 2, 2, 31);
}

#define ROGUE_VDMCTRL_STREAM_RETURN_length      1
#define ROGUE_VDMCTRL_STREAM_RETURN_header      \
    .block_type                          = ROGUE_VDMCTRL_BLOCK_TYPE_STREAM_RETURN

struct ROGUE_VDMCTRL_STREAM_RETURN {
    enum ROGUE_VDMCTRL_BLOCK_TYPE        block_type;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_STREAM_RETURN_pack(__attribute__((unused)) void * restrict dst,
                                 __attribute__((unused)) const struct ROGUE_VDMCTRL_STREAM_RETURN * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->block_type, 29, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_STREAM_RETURN_unpack(__attribute__((unused)) const void * restrict src,
                                   __attribute__((unused)) struct ROGUE_VDMCTRL_STREAM_RETURN * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->block_type = __pvr_uint_unpack(dw[0], 29, 31);
}

#define ROGUE_VDMCTRL_STREAM_TERMINATE_length      1
#define ROGUE_VDMCTRL_STREAM_TERMINATE_header   \
    .block_type                          = ROGUE_VDMCTRL_BLOCK_TYPE_STREAM_TERMINATE

struct ROGUE_VDMCTRL_STREAM_TERMINATE {
    enum ROGUE_VDMCTRL_BLOCK_TYPE        block_type;
    bool                                 context;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_STREAM_TERMINATE_pack(__attribute__((unused)) void * restrict dst,
                                    __attribute__((unused)) const struct ROGUE_VDMCTRL_STREAM_TERMINATE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->block_type, 29, 31) |
      __pvr_uint(values->context, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_STREAM_TERMINATE_unpack(__attribute__((unused)) const void * restrict src,
                                      __attribute__((unused)) struct ROGUE_VDMCTRL_STREAM_TERMINATE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->block_type = __pvr_uint_unpack(dw[0], 29, 31);
    values->context = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_VDMCTRL_VDM_STATE0_length        1
#define ROGUE_VDMCTRL_VDM_STATE0_header         \
    .block_type                          = ROGUE_VDMCTRL_BLOCK_TYPE_VDM_STATE_UPDATE

struct ROGUE_VDMCTRL_VDM_STATE0 {
    enum ROGUE_VDMCTRL_BLOCK_TYPE        block_type;
    bool                                 cut_index_present;
    bool                                 vs_data_addr_present;
    bool                                 vs_other_present;
    bool                                 ds_present;
    bool                                 gs_present;
    bool                                 hs_present;
    uint32_t                             cam_size;
    enum ROGUE_VDMCTRL_UVS_SCRATCH_SIZE_SELECT uvs_scratch_size_select;
    bool                                 cut_index_enable;
    bool                                 tess_enable;
    bool                                 gs_enable;
    enum ROGUE_VDMCTRL_FLATSHADE_CONTROL flatshade_control;
    bool                                 generate_primitive_id;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE0_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_VDM_STATE0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->block_type, 29, 31) |
      __pvr_uint(values->cut_index_present, 28, 28) |
      __pvr_uint(values->vs_data_addr_present, 27, 27) |
      __pvr_uint(values->vs_other_present, 26, 26) |
      __pvr_uint(values->ds_present, 24, 24) |
      __pvr_uint(values->gs_present, 23, 23) |
      __pvr_uint(values->hs_present, 22, 22) |
      __pvr_uint(values->cam_size, 7, 14) |
      __pvr_uint(values->uvs_scratch_size_select, 6, 6) |
      __pvr_uint(values->cut_index_enable, 5, 5) |
      __pvr_uint(values->tess_enable, 4, 4) |
      __pvr_uint(values->gs_enable, 3, 3) |
      __pvr_uint(values->flatshade_control, 1, 2) |
      __pvr_uint(values->generate_primitive_id, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE0_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_VDM_STATE0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->block_type = __pvr_uint_unpack(dw[0], 29, 31);
    values->cut_index_present = __pvr_uint_unpack(dw[0], 28, 28);
    values->vs_data_addr_present = __pvr_uint_unpack(dw[0], 27, 27);
    values->vs_other_present = __pvr_uint_unpack(dw[0], 26, 26);
    values->ds_present = __pvr_uint_unpack(dw[0], 24, 24);
    values->gs_present = __pvr_uint_unpack(dw[0], 23, 23);
    values->hs_present = __pvr_uint_unpack(dw[0], 22, 22);
    values->cam_size = __pvr_uint_unpack(dw[0], 7, 14);
    values->uvs_scratch_size_select = __pvr_uint_unpack(dw[0], 6, 6);
    values->cut_index_enable = __pvr_uint_unpack(dw[0], 5, 5);
    values->tess_enable = __pvr_uint_unpack(dw[0], 4, 4);
    values->gs_enable = __pvr_uint_unpack(dw[0], 3, 3);
    values->flatshade_control = __pvr_uint_unpack(dw[0], 1, 2);
    values->generate_primitive_id = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_VDMCTRL_VDM_STATE1_length        1
#define ROGUE_VDMCTRL_VDM_STATE1_header         \


struct ROGUE_VDMCTRL_VDM_STATE1 {
    uint32_t                             cut_index;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE1_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_VDM_STATE1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->cut_index, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE1_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_VDM_STATE1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->cut_index = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_VDMCTRL_VDM_STATE2_length        1
#define ROGUE_VDMCTRL_VDM_STATE2_header         \


/* Helper macros for vs_pds_data_base_addr */
#define ROGUE_VDMCTRL_VDM_STATE2_VS_PDS_DATA_BASE_ADDR_ALIGNMENT 16

struct ROGUE_VDMCTRL_VDM_STATE2 {
    __pvr_address_type                   vs_pds_data_base_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE2_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_VDM_STATE2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->vs_pds_data_base_addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE2_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_VDM_STATE2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->vs_pds_data_base_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_VDMCTRL_VDM_STATE3_length        1
#define ROGUE_VDMCTRL_VDM_STATE3_header         \


/* Helper macros for vs_pds_code_base_addr */
#define ROGUE_VDMCTRL_VDM_STATE3_VS_PDS_CODE_BASE_ADDR_ALIGNMENT 16

struct ROGUE_VDMCTRL_VDM_STATE3 {
    __pvr_address_type                   vs_pds_code_base_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE3_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_VDM_STATE3 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->vs_pds_code_base_addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE3_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_VDM_STATE3 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->vs_pds_code_base_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_VDMCTRL_VDM_STATE4_length        1
#define ROGUE_VDMCTRL_VDM_STATE4_header         \


/* Helper macros for vs_output_size */
#define ROGUE_VDMCTRL_VDM_STATE4_VS_OUTPUT_SIZE_UNIT_SIZE 4

struct ROGUE_VDMCTRL_VDM_STATE4 {
    uint32_t                             vs_output_size;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE4_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_VDM_STATE4 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->vs_output_size, 0, 7);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE4_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_VDM_STATE4 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->vs_output_size = __pvr_uint_unpack(dw[0], 0, 7);
}

#define ROGUE_VDMCTRL_VDM_STATE5_length        1
#define ROGUE_VDMCTRL_VDM_STATE5_header         \


/* Helper macros for vs_usc_common_size */
#define ROGUE_VDMCTRL_VDM_STATE5_VS_USC_COMMON_SIZE_UNIT_SIZE 64

/* Helper macros for vs_usc_unified_size */
#define ROGUE_VDMCTRL_VDM_STATE5_VS_USC_UNIFIED_SIZE_UNIT_SIZE 16

/* Helper macros for vs_pds_temp_size */
#define ROGUE_VDMCTRL_VDM_STATE5_VS_PDS_TEMP_SIZE_UNIT_SIZE 16

/* Helper macros for vs_pds_data_size */
#define ROGUE_VDMCTRL_VDM_STATE5_VS_PDS_DATA_SIZE_UNIT_SIZE 16

struct ROGUE_VDMCTRL_VDM_STATE5 {
    bool                                 vs_max_instances_ext;
    uint32_t                             vs_max_instances;
    uint32_t                             vs_usc_common_size;
    uint32_t                             vs_usc_unified_size;
    uint32_t                             vs_pds_temp_size;
    uint32_t                             vs_pds_data_size;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE5_pack(__attribute__((unused)) void * restrict dst,
                              __attribute__((unused)) const struct ROGUE_VDMCTRL_VDM_STATE5 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->vs_max_instances_ext, 31, 31) |
      __pvr_uint(values->vs_max_instances, 25, 29) |
      __pvr_uint(values->vs_usc_common_size, 16, 24) |
      __pvr_uint(values->vs_usc_unified_size, 10, 15) |
      __pvr_uint(values->vs_pds_temp_size, 6, 9) |
      __pvr_uint(values->vs_pds_data_size, 0, 5);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_VDM_STATE5_unpack(__attribute__((unused)) const void * restrict src,
                                __attribute__((unused)) struct ROGUE_VDMCTRL_VDM_STATE5 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->vs_max_instances_ext = __pvr_uint_unpack(dw[0], 31, 31);
    values->vs_max_instances = __pvr_uint_unpack(dw[0], 25, 29);
    values->vs_usc_common_size = __pvr_uint_unpack(dw[0], 16, 24);
    values->vs_usc_unified_size = __pvr_uint_unpack(dw[0], 10, 15);
    values->vs_pds_temp_size = __pvr_uint_unpack(dw[0], 6, 9);
    values->vs_pds_data_size = __pvr_uint_unpack(dw[0], 0, 5);
}

#define ROGUE_VDMCTRL_INDEX_LIST0_length       1
#define ROGUE_VDMCTRL_INDEX_LIST0_header        \
    .block_type                          = ROGUE_VDMCTRL_BLOCK_TYPE_INDEX_LIST

/* Helper macros for index_base_addrmsb */
#define ROGUE_VDMCTRL_INDEX_LIST0_INDEX_BASE_ADDRMSB_ALIGNMENT 4294967296

struct ROGUE_VDMCTRL_INDEX_LIST0 {
    enum ROGUE_VDMCTRL_BLOCK_TYPE        block_type;
    bool                                 index_addr_present;
    bool                                 index_count_present;
    bool                                 index_instance_count_present;
    bool                                 index_offset_present;
    bool                                 start_present;
    bool                                 indirect_addr_present;
    bool                                 split_count_present;
/* if VDM_DEGENERATE_CULLING is supported use: */
    bool                                 degen_cull_enable;
/* endif VDM_DEGENERATE_CULLING */
    enum ROGUE_VDMCTRL_INDEX_SIZE        index_size;
    uint32_t                             patch_count;
    enum ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY primitive_topology;
    __pvr_address_type                   index_base_addrmsb;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST0_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    const uint32_t v0 =
      __pvr_uint(values->block_type, 29, 31) |
      __pvr_uint(values->index_addr_present, 28, 28) |
      __pvr_uint(values->index_count_present, 27, 27) |
      __pvr_uint(values->index_instance_count_present, 26, 26) |
      __pvr_uint(values->index_offset_present, 25, 25) |
      __pvr_uint(values->start_present, 24, 24) |
      __pvr_uint(values->indirect_addr_present, 23, 23) |
      __pvr_uint(values->split_count_present, 22, 22) |
      __pvr_uint(values->degen_cull_enable, 19, 19) |
      __pvr_uint(values->index_size, 17, 18) |
      __pvr_uint(values->patch_count, 12, 16) |
      __pvr_uint(values->primitive_topology, 8, 11);
    dw[0] = __pvr_address(values->index_base_addrmsb, 32, 0, 7) | v0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST0_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->block_type = __pvr_uint_unpack(dw[0], 29, 31);
    values->index_addr_present = __pvr_uint_unpack(dw[0], 28, 28);
    values->index_count_present = __pvr_uint_unpack(dw[0], 27, 27);
    values->index_instance_count_present = __pvr_uint_unpack(dw[0], 26, 26);
    values->index_offset_present = __pvr_uint_unpack(dw[0], 25, 25);
    values->start_present = __pvr_uint_unpack(dw[0], 24, 24);
    values->indirect_addr_present = __pvr_uint_unpack(dw[0], 23, 23);
    values->split_count_present = __pvr_uint_unpack(dw[0], 22, 22);
    values->degen_cull_enable = __pvr_uint_unpack(dw[0], 19, 19);
    values->index_size = __pvr_uint_unpack(dw[0], 17, 18);
    values->patch_count = __pvr_uint_unpack(dw[0], 12, 16);
    values->primitive_topology = __pvr_uint_unpack(dw[0], 8, 11);
    values->index_base_addrmsb = __pvr_address_unpack(dw[0], 32, 0, 7);
}

#define ROGUE_VDMCTRL_INDEX_LIST1_length       1
#define ROGUE_VDMCTRL_INDEX_LIST1_header        \


/* Helper macros for index_base_addrlsb */
#define ROGUE_VDMCTRL_INDEX_LIST1_INDEX_BASE_ADDRLSB_ALIGNMENT 1

struct ROGUE_VDMCTRL_INDEX_LIST1 {
    __pvr_address_type                   index_base_addrlsb;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST1_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->index_base_addrlsb, 0, 0, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST1_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->index_base_addrlsb = __pvr_address_unpack(dw[0], 0, 0, 31);
}

#define ROGUE_VDMCTRL_INDEX_LIST2_length       1
#define ROGUE_VDMCTRL_INDEX_LIST2_header        \


struct ROGUE_VDMCTRL_INDEX_LIST2 {
    uint32_t                             index_count;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST2_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->index_count, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST2_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->index_count = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_VDMCTRL_INDEX_LIST3_length       1
#define ROGUE_VDMCTRL_INDEX_LIST3_header        \


struct ROGUE_VDMCTRL_INDEX_LIST3 {
    uint32_t                             instance_count;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST3_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST3 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->instance_count, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST3_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST3 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->instance_count = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_VDMCTRL_INDEX_LIST4_length       1
#define ROGUE_VDMCTRL_INDEX_LIST4_header        \


struct ROGUE_VDMCTRL_INDEX_LIST4 {
    uint32_t                             index_offset;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST4_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST4 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->index_offset, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST4_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST4 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->index_offset = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_VDMCTRL_INDEX_LIST5_length       1
#define ROGUE_VDMCTRL_INDEX_LIST5_header        \


struct ROGUE_VDMCTRL_INDEX_LIST5 {
    uint32_t                             start_index;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST5_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST5 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->start_index, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST5_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST5 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->start_index = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_VDMCTRL_INDEX_LIST6_length       1
#define ROGUE_VDMCTRL_INDEX_LIST6_header        \


struct ROGUE_VDMCTRL_INDEX_LIST6 {
    uint32_t                             start_instance;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST6_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST6 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->start_instance, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST6_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST6 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->start_instance = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_VDMCTRL_INDEX_LIST7_length       1
#define ROGUE_VDMCTRL_INDEX_LIST7_header        \


/* Helper macros for indirect_base_addrmsb */
#define ROGUE_VDMCTRL_INDEX_LIST7_INDIRECT_BASE_ADDRMSB_ALIGNMENT 4294967296

struct ROGUE_VDMCTRL_INDEX_LIST7 {
    __pvr_address_type                   indirect_base_addrmsb;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST7_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST7 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->indirect_base_addrmsb, 32, 0, 7) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST7_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST7 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->indirect_base_addrmsb = __pvr_address_unpack(dw[0], 32, 0, 7);
}

#define ROGUE_VDMCTRL_INDEX_LIST8_length       1
#define ROGUE_VDMCTRL_INDEX_LIST8_header        \


/* Helper macros for indirect_base_addrlsb */
#define ROGUE_VDMCTRL_INDEX_LIST8_INDIRECT_BASE_ADDRLSB_ALIGNMENT 4

struct ROGUE_VDMCTRL_INDEX_LIST8 {
    __pvr_address_type                   indirect_base_addrlsb;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST8_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST8 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->indirect_base_addrlsb, 2, 2, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST8_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST8 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->indirect_base_addrlsb = __pvr_address_unpack(dw[0], 2, 2, 31);
}

#define ROGUE_VDMCTRL_INDEX_LIST9_length       1
#define ROGUE_VDMCTRL_INDEX_LIST9_header        \


struct ROGUE_VDMCTRL_INDEX_LIST9 {
    uint32_t                             split_count;
};

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST9_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_VDMCTRL_INDEX_LIST9 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->split_count, 0, 15);
}

static inline __attribute__((always_inline)) void
ROGUE_VDMCTRL_INDEX_LIST9_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_VDMCTRL_INDEX_LIST9 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->split_count = __pvr_uint_unpack(dw[0], 0, 15);
}

#endif /* VDM_H */
