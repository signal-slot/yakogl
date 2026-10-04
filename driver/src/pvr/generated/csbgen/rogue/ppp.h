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

#ifndef PPP_H
#define PPP_H

#include "csbgen/pvr_packet_helpers.h"

#ifndef __OPENCL_VERSION__
#define __constant
#endif


#define ROGUE_TA_REGION_CLIP_MAX                 511

enum ROGUE_TA_CLIP_MODE {
    ROGUE_TA_CLIP_MODE_NO_FRONT_OR_REAR  =      0,
    ROGUE_TA_CLIP_MODE_FRONT_REAR        =      1,
    ROGUE_TA_CLIP_MODE_FRONT_REAR_DEPTH  =      2,
};

static __constant const char *
ROGUE_TA_CLIP_MODE_to_str(const enum ROGUE_TA_CLIP_MODE value)
{
    switch (value) {
    case ROGUE_TA_CLIP_MODE_NO_FRONT_OR_REAR: return "NO_FRONT_OR_REAR";
    case ROGUE_TA_CLIP_MODE_FRONT_REAR: return "FRONT_REAR";
    case ROGUE_TA_CLIP_MODE_FRONT_REAR_DEPTH: return "FRONT_REAR_DEPTH";
    default: return NULL;
    }
}

enum ROGUE_TA_CMPMODE {
    ROGUE_TA_CMPMODE_NEVER               =      0,
    ROGUE_TA_CMPMODE_LESS                =      1,
    ROGUE_TA_CMPMODE_EQUAL               =      2,
    ROGUE_TA_CMPMODE_LESS_OR_EQUAL       =      3,
    ROGUE_TA_CMPMODE_GREATER             =      4,
    ROGUE_TA_CMPMODE_NOT_EQUAL           =      5,
    ROGUE_TA_CMPMODE_GREATER_OR_EQUAL    =      6,
    ROGUE_TA_CMPMODE_ALWAYS              =      7,
};

static __constant const char *
ROGUE_TA_CMPMODE_to_str(const enum ROGUE_TA_CMPMODE value)
{
    switch (value) {
    case ROGUE_TA_CMPMODE_NEVER: return "NEVER";
    case ROGUE_TA_CMPMODE_LESS: return "LESS";
    case ROGUE_TA_CMPMODE_EQUAL: return "EQUAL";
    case ROGUE_TA_CMPMODE_LESS_OR_EQUAL: return "LESS_OR_EQUAL";
    case ROGUE_TA_CMPMODE_GREATER: return "GREATER";
    case ROGUE_TA_CMPMODE_NOT_EQUAL: return "NOT_EQUAL";
    case ROGUE_TA_CMPMODE_GREATER_OR_EQUAL: return "GREATER_OR_EQUAL";
    case ROGUE_TA_CMPMODE_ALWAYS: return "ALWAYS";
    default: return NULL;
    }
}

enum ROGUE_TA_CULLMODE {
    ROGUE_TA_CULLMODE_NO_CULLING         =      0,
    ROGUE_TA_CULLMODE_CULL_CW            =      1,
    ROGUE_TA_CULLMODE_CULL_CCW           =      2,
};

static __constant const char *
ROGUE_TA_CULLMODE_to_str(const enum ROGUE_TA_CULLMODE value)
{
    switch (value) {
    case ROGUE_TA_CULLMODE_NO_CULLING: return "NO_CULLING";
    case ROGUE_TA_CULLMODE_CULL_CW: return "CULL_CW";
    case ROGUE_TA_CULLMODE_CULL_CCW: return "CULL_CCW";
    default: return NULL;
    }
}

enum ROGUE_TA_FLATSHADE {
    ROGUE_TA_FLATSHADE_VTX_VERTEX_0      =      1,
    ROGUE_TA_FLATSHADE_VTX_VERTEX_1      =      2,
    ROGUE_TA_FLATSHADE_VTX_VERTEX_2      =      3,
};

static __constant const char *
ROGUE_TA_FLATSHADE_to_str(const enum ROGUE_TA_FLATSHADE value)
{
    switch (value) {
    case ROGUE_TA_FLATSHADE_VTX_VERTEX_0: return "VTX_VERTEX_0";
    case ROGUE_TA_FLATSHADE_VTX_VERTEX_1: return "VTX_VERTEX_1";
    case ROGUE_TA_FLATSHADE_VTX_VERTEX_2: return "VTX_VERTEX_2";
    default: return NULL;
    }
}

enum ROGUE_TA_GS_OUTPUT_TOPOLOGY {
    ROGUE_TA_GS_OUTPUT_TOPOLOGY_POINT_LIST =      0,
    ROGUE_TA_GS_OUTPUT_TOPOLOGY_LINE_STRIP =      1,
    ROGUE_TA_GS_OUTPUT_TOPOLOGY_TRI_STRIP =      2,
};

static __constant const char *
ROGUE_TA_GS_OUTPUT_TOPOLOGY_to_str(const enum ROGUE_TA_GS_OUTPUT_TOPOLOGY value)
{
    switch (value) {
    case ROGUE_TA_GS_OUTPUT_TOPOLOGY_POINT_LIST: return "POINT_LIST";
    case ROGUE_TA_GS_OUTPUT_TOPOLOGY_LINE_STRIP: return "LINE_STRIP";
    case ROGUE_TA_GS_OUTPUT_TOPOLOGY_TRI_STRIP: return "TRI_STRIP";
    default: return NULL;
    }
}

enum ROGUE_TA_ISPB_STENCILOP {
    ROGUE_TA_ISPB_STENCILOP_KEEP         =      0,
    ROGUE_TA_ISPB_STENCILOP_ZERO         =      1,
    ROGUE_TA_ISPB_STENCILOP_REPLACE      =      2,
    ROGUE_TA_ISPB_STENCILOP_INCREMENT_SATURATE =      3,
    ROGUE_TA_ISPB_STENCILOP_DECREMENT_SATURATE =      4,
    ROGUE_TA_ISPB_STENCILOP_INVERT       =      5,
    ROGUE_TA_ISPB_STENCILOP_INCREMENT    =      6,
    ROGUE_TA_ISPB_STENCILOP_DECREMENT    =      7,
};

static __constant const char *
ROGUE_TA_ISPB_STENCILOP_to_str(const enum ROGUE_TA_ISPB_STENCILOP value)
{
    switch (value) {
    case ROGUE_TA_ISPB_STENCILOP_KEEP: return "KEEP";
    case ROGUE_TA_ISPB_STENCILOP_ZERO: return "ZERO";
    case ROGUE_TA_ISPB_STENCILOP_REPLACE: return "REPLACE";
    case ROGUE_TA_ISPB_STENCILOP_INCREMENT_SATURATE: return "INCREMENT_SATURATE";
    case ROGUE_TA_ISPB_STENCILOP_DECREMENT_SATURATE: return "DECREMENT_SATURATE";
    case ROGUE_TA_ISPB_STENCILOP_INVERT: return "INVERT";
    case ROGUE_TA_ISPB_STENCILOP_INCREMENT: return "INCREMENT";
    case ROGUE_TA_ISPB_STENCILOP_DECREMENT: return "DECREMENT";
    default: return NULL;
    }
}

enum ROGUE_TA_OBJTYPE {
    ROGUE_TA_OBJTYPE_TRIANGLE            =      0,
    ROGUE_TA_OBJTYPE_LINE                =      1,
    ROGUE_TA_OBJTYPE_SPRITE_10UV         =      2,
    ROGUE_TA_OBJTYPE_SPRITE_UV           =      3,
    ROGUE_TA_OBJTYPE_SPRITE_01UV         =      4,
    ROGUE_TA_OBJTYPE_LINE_FILLED_TRIANGLE =      5,
    ROGUE_TA_OBJTYPE_POINT_FILLED_TRIANGLE =      6,
    ROGUE_TA_OBJTYPE_TESSELLATED_OBJECT_NO_GS =      7,
    ROGUE_TA_OBJTYPE_TESSELLATED_OBJECT_WITH_GS =      8,
};

static __constant const char *
ROGUE_TA_OBJTYPE_to_str(const enum ROGUE_TA_OBJTYPE value)
{
    switch (value) {
    case ROGUE_TA_OBJTYPE_TRIANGLE: return "TRIANGLE";
    case ROGUE_TA_OBJTYPE_LINE: return "LINE";
    case ROGUE_TA_OBJTYPE_SPRITE_10UV: return "SPRITE_10UV";
    case ROGUE_TA_OBJTYPE_SPRITE_UV: return "SPRITE_UV";
    case ROGUE_TA_OBJTYPE_SPRITE_01UV: return "SPRITE_01UV";
    case ROGUE_TA_OBJTYPE_LINE_FILLED_TRIANGLE: return "LINE_FILLED_TRIANGLE";
    case ROGUE_TA_OBJTYPE_POINT_FILLED_TRIANGLE: return "POINT_FILLED_TRIANGLE";
    case ROGUE_TA_OBJTYPE_TESSELLATED_OBJECT_NO_GS: return "TESSELLATED_OBJECT_NO_GS";
    case ROGUE_TA_OBJTYPE_TESSELLATED_OBJECT_WITH_GS: return "TESSELLATED_OBJECT_WITH_GS";
    default: return NULL;
    }
}

enum ROGUE_TA_PASSTYPE {
    ROGUE_TA_PASSTYPE_OPAQUE             =      0,
    ROGUE_TA_PASSTYPE_TRANSLUCENT        =      1,
    ROGUE_TA_PASSTYPE_PUNCH_THROUGH      =      2,
    ROGUE_TA_PASSTYPE_VIEWPORT_OBJECT    =      3,
    ROGUE_TA_PASSTYPE_FAST_PUNCH_THROUGH =      4,
    ROGUE_TA_PASSTYPE_DEPTH_FEEDBACK     =      5,
    ROGUE_TA_PASSTYPE_ANTI_ALIASED       =      6,
};

static __constant const char *
ROGUE_TA_PASSTYPE_to_str(const enum ROGUE_TA_PASSTYPE value)
{
    switch (value) {
    case ROGUE_TA_PASSTYPE_OPAQUE: return "OPAQUE";
    case ROGUE_TA_PASSTYPE_TRANSLUCENT: return "TRANSLUCENT";
    case ROGUE_TA_PASSTYPE_PUNCH_THROUGH: return "PUNCH_THROUGH";
    case ROGUE_TA_PASSTYPE_VIEWPORT_OBJECT: return "VIEWPORT_OBJECT";
    case ROGUE_TA_PASSTYPE_FAST_PUNCH_THROUGH: return "FAST_PUNCH_THROUGH";
    case ROGUE_TA_PASSTYPE_DEPTH_FEEDBACK: return "DEPTH_FEEDBACK";
    case ROGUE_TA_PASSTYPE_ANTI_ALIASED: return "ANTI_ALIASED";
    default: return NULL;
    }
}

enum ROGUE_TA_REGION_CLIP_MODE {
    ROGUE_TA_REGION_CLIP_MODE_NONE       =      0,
    ROGUE_TA_REGION_CLIP_MODE_OUTSIDE    =      1,
};

static __constant const char *
ROGUE_TA_REGION_CLIP_MODE_to_str(const enum ROGUE_TA_REGION_CLIP_MODE value)
{
    switch (value) {
    case ROGUE_TA_REGION_CLIP_MODE_NONE: return "NONE";
    case ROGUE_TA_REGION_CLIP_MODE_OUTSIDE: return "OUTSIDE";
    default: return NULL;
    }
}

#define ROGUE_TA_STATE_HEADER_length           1
#define ROGUE_TA_STATE_HEADER_header            \


struct ROGUE_TA_STATE_HEADER {
    bool                                 not_final_term;
    bool                                 pres_terminate;
    bool                                 context_switch;
    bool                                 pres_stream_out_program;
    bool                                 pres_stream_out_size;
    bool                                 pres_ppp_ctrl;
    bool                                 pres_varying_word2;
    bool                                 pres_varying_word1;
    bool                                 pres_varying_word0;
    bool                                 pres_outselects;
    bool                                 pres_wclamp;
    uint32_t                             view_port_count;
    bool                                 pres_viewport;
    bool                                 pres_region_clip;
    bool                                 pres_pds_state_ptr3;
    bool                                 pres_pds_state_ptr2;
    bool                                 pres_pds_state_ptr1;
    bool                                 pres_pds_state_ptr0;
    bool                                 pres_ispctl_dbsc;
    bool                                 pres_ispctl_bb;
    bool                                 pres_ispctl_ba;
    bool                                 pres_ispctl_fb;
    bool                                 pres_ispctl_fa;
    bool                                 pres_ispctl;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_HEADER_pack(__attribute__((unused)) void * restrict dst,
                           __attribute__((unused)) const struct ROGUE_TA_STATE_HEADER * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->not_final_term, 26, 26) |
      __pvr_uint(values->pres_terminate, 25, 25) |
      __pvr_uint(values->context_switch, 24, 24) |
      __pvr_uint(values->pres_stream_out_program, 23, 23) |
      __pvr_uint(values->pres_stream_out_size, 22, 22) |
      __pvr_uint(values->pres_ppp_ctrl, 21, 21) |
      __pvr_uint(values->pres_varying_word2, 20, 20) |
      __pvr_uint(values->pres_varying_word1, 19, 19) |
      __pvr_uint(values->pres_varying_word0, 18, 18) |
      __pvr_uint(values->pres_outselects, 17, 17) |
      __pvr_uint(values->pres_wclamp, 16, 16) |
      __pvr_uint(values->view_port_count, 12, 15) |
      __pvr_uint(values->pres_viewport, 11, 11) |
      __pvr_uint(values->pres_region_clip, 10, 10) |
      __pvr_uint(values->pres_pds_state_ptr3, 9, 9) |
      __pvr_uint(values->pres_pds_state_ptr2, 8, 8) |
      __pvr_uint(values->pres_pds_state_ptr1, 7, 7) |
      __pvr_uint(values->pres_pds_state_ptr0, 6, 6) |
      __pvr_uint(values->pres_ispctl_dbsc, 5, 5) |
      __pvr_uint(values->pres_ispctl_bb, 4, 4) |
      __pvr_uint(values->pres_ispctl_ba, 3, 3) |
      __pvr_uint(values->pres_ispctl_fb, 2, 2) |
      __pvr_uint(values->pres_ispctl_fa, 1, 1) |
      __pvr_uint(values->pres_ispctl, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_HEADER_unpack(__attribute__((unused)) const void * restrict src,
                             __attribute__((unused)) struct ROGUE_TA_STATE_HEADER * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->not_final_term = __pvr_uint_unpack(dw[0], 26, 26);
    values->pres_terminate = __pvr_uint_unpack(dw[0], 25, 25);
    values->context_switch = __pvr_uint_unpack(dw[0], 24, 24);
    values->pres_stream_out_program = __pvr_uint_unpack(dw[0], 23, 23);
    values->pres_stream_out_size = __pvr_uint_unpack(dw[0], 22, 22);
    values->pres_ppp_ctrl = __pvr_uint_unpack(dw[0], 21, 21);
    values->pres_varying_word2 = __pvr_uint_unpack(dw[0], 20, 20);
    values->pres_varying_word1 = __pvr_uint_unpack(dw[0], 19, 19);
    values->pres_varying_word0 = __pvr_uint_unpack(dw[0], 18, 18);
    values->pres_outselects = __pvr_uint_unpack(dw[0], 17, 17);
    values->pres_wclamp = __pvr_uint_unpack(dw[0], 16, 16);
    values->view_port_count = __pvr_uint_unpack(dw[0], 12, 15);
    values->pres_viewport = __pvr_uint_unpack(dw[0], 11, 11);
    values->pres_region_clip = __pvr_uint_unpack(dw[0], 10, 10);
    values->pres_pds_state_ptr3 = __pvr_uint_unpack(dw[0], 9, 9);
    values->pres_pds_state_ptr2 = __pvr_uint_unpack(dw[0], 8, 8);
    values->pres_pds_state_ptr1 = __pvr_uint_unpack(dw[0], 7, 7);
    values->pres_pds_state_ptr0 = __pvr_uint_unpack(dw[0], 6, 6);
    values->pres_ispctl_dbsc = __pvr_uint_unpack(dw[0], 5, 5);
    values->pres_ispctl_bb = __pvr_uint_unpack(dw[0], 4, 4);
    values->pres_ispctl_ba = __pvr_uint_unpack(dw[0], 3, 3);
    values->pres_ispctl_fb = __pvr_uint_unpack(dw[0], 2, 2);
    values->pres_ispctl_fa = __pvr_uint_unpack(dw[0], 1, 1);
    values->pres_ispctl = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_TA_STATE_ISPCTL_length           1
#define ROGUE_TA_STATE_ISPCTL_header            \


struct ROGUE_TA_STATE_ISPCTL {
    uint32_t                             validid;
    uint32_t                             upass;
    bool                                 tagwritedisable;
    bool                                 ovgmtestdisable;
    bool                                 two_sided;
    bool                                 bpres;
    bool                                 dbenable;
    bool                                 scenable;
    bool                                 vistest;
    bool                                 visbool;
    uint32_t                             visreg;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_ISPCTL_pack(__attribute__((unused)) void * restrict dst,
                           __attribute__((unused)) const struct ROGUE_TA_STATE_ISPCTL * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->validid, 26, 31) |
      __pvr_uint(values->upass, 22, 25) |
      __pvr_uint(values->tagwritedisable, 21, 21) |
      __pvr_uint(values->ovgmtestdisable, 20, 20) |
      __pvr_uint(values->two_sided, 19, 19) |
      __pvr_uint(values->bpres, 18, 18) |
      __pvr_uint(values->dbenable, 17, 17) |
      __pvr_uint(values->scenable, 16, 16) |
      __pvr_uint(values->vistest, 15, 15) |
      __pvr_uint(values->visbool, 14, 14) |
      __pvr_uint(values->visreg, 0, 13);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_ISPCTL_unpack(__attribute__((unused)) const void * restrict src,
                             __attribute__((unused)) struct ROGUE_TA_STATE_ISPCTL * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->validid = __pvr_uint_unpack(dw[0], 26, 31);
    values->upass = __pvr_uint_unpack(dw[0], 22, 25);
    values->tagwritedisable = __pvr_uint_unpack(dw[0], 21, 21);
    values->ovgmtestdisable = __pvr_uint_unpack(dw[0], 20, 20);
    values->two_sided = __pvr_uint_unpack(dw[0], 19, 19);
    values->bpres = __pvr_uint_unpack(dw[0], 18, 18);
    values->dbenable = __pvr_uint_unpack(dw[0], 17, 17);
    values->scenable = __pvr_uint_unpack(dw[0], 16, 16);
    values->vistest = __pvr_uint_unpack(dw[0], 15, 15);
    values->visbool = __pvr_uint_unpack(dw[0], 14, 14);
    values->visreg = __pvr_uint_unpack(dw[0], 0, 13);
}

#define ROGUE_TA_STATE_ISPA_length             1
#define ROGUE_TA_STATE_ISPA_header              \


/* Helper macros for pointlinewidth */
#define ROGUE_TA_STATE_ISPA_POINTLINEWIDTH_SIZE_MAX 255

/* Helper macros for sref */
#define ROGUE_TA_STATE_ISPA_SREF_SIZE_MAX        255

struct ROGUE_TA_STATE_ISPA {
    enum ROGUE_TA_OBJTYPE                objtype;
    enum ROGUE_TA_PASSTYPE               passtype;
    bool                                 ovgvispassmaskop;
    bool                                 maskval;
    bool                                 dwritedisable;
    bool                                 dfbztestenable;
    enum ROGUE_TA_CMPMODE                dcmpmode;
    bool                                 linefilllastpixel;
    uint32_t                             pointlinewidth;
    uint32_t                             sref;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_ISPA_pack(__attribute__((unused)) void * restrict dst,
                         __attribute__((unused)) const struct ROGUE_TA_STATE_ISPA * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->objtype, 28, 31) |
      __pvr_uint(values->passtype, 24, 26) |
      __pvr_uint(values->ovgvispassmaskop, 23, 23) |
      __pvr_uint(values->maskval, 22, 22) |
      __pvr_uint(values->dwritedisable, 21, 21) |
      __pvr_uint(values->dfbztestenable, 20, 20) |
      __pvr_uint(values->dcmpmode, 17, 19) |
      __pvr_uint(values->linefilllastpixel, 16, 16) |
      __pvr_uint(values->pointlinewidth, 8, 15) |
      __pvr_uint(values->sref, 0, 7);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_ISPA_unpack(__attribute__((unused)) const void * restrict src,
                           __attribute__((unused)) struct ROGUE_TA_STATE_ISPA * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->objtype = __pvr_uint_unpack(dw[0], 28, 31);
    values->passtype = __pvr_uint_unpack(dw[0], 24, 26);
    values->ovgvispassmaskop = __pvr_uint_unpack(dw[0], 23, 23);
    values->maskval = __pvr_uint_unpack(dw[0], 22, 22);
    values->dwritedisable = __pvr_uint_unpack(dw[0], 21, 21);
    values->dfbztestenable = __pvr_uint_unpack(dw[0], 20, 20);
    values->dcmpmode = __pvr_uint_unpack(dw[0], 17, 19);
    values->linefilllastpixel = __pvr_uint_unpack(dw[0], 16, 16);
    values->pointlinewidth = __pvr_uint_unpack(dw[0], 8, 15);
    values->sref = __pvr_uint_unpack(dw[0], 0, 7);
}

#define ROGUE_TA_STATE_ISPB_length             1
#define ROGUE_TA_STATE_ISPB_header              \


struct ROGUE_TA_STATE_ISPB {
    enum ROGUE_TA_CMPMODE                scmpmode;
    enum ROGUE_TA_ISPB_STENCILOP         sop1;
    enum ROGUE_TA_ISPB_STENCILOP         sop2;
    enum ROGUE_TA_ISPB_STENCILOP         sop3;
    uint32_t                             scmpmask;
    uint32_t                             swmask;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_ISPB_pack(__attribute__((unused)) void * restrict dst,
                         __attribute__((unused)) const struct ROGUE_TA_STATE_ISPB * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->scmpmode, 25, 27) |
      __pvr_uint(values->sop1, 22, 24) |
      __pvr_uint(values->sop2, 19, 21) |
      __pvr_uint(values->sop3, 16, 18) |
      __pvr_uint(values->scmpmask, 8, 15) |
      __pvr_uint(values->swmask, 0, 7);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_ISPB_unpack(__attribute__((unused)) const void * restrict src,
                           __attribute__((unused)) struct ROGUE_TA_STATE_ISPB * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->scmpmode = __pvr_uint_unpack(dw[0], 25, 27);
    values->sop1 = __pvr_uint_unpack(dw[0], 22, 24);
    values->sop2 = __pvr_uint_unpack(dw[0], 19, 21);
    values->sop3 = __pvr_uint_unpack(dw[0], 16, 18);
    values->scmpmask = __pvr_uint_unpack(dw[0], 8, 15);
    values->swmask = __pvr_uint_unpack(dw[0], 0, 7);
}

#define ROGUE_TA_REGION_CLIP0_length           1
#define ROGUE_TA_REGION_CLIP0_header            \


struct ROGUE_TA_REGION_CLIP0 {
    enum ROGUE_TA_REGION_CLIP_MODE       mode;
    uint32_t                             left;
    uint32_t                             right;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_REGION_CLIP0_pack(__attribute__((unused)) void * restrict dst,
                           __attribute__((unused)) const struct ROGUE_TA_REGION_CLIP0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->mode, 31, 31) |
      __pvr_uint(values->left, 16, 24) |
      __pvr_uint(values->right, 0, 8);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_REGION_CLIP0_unpack(__attribute__((unused)) const void * restrict src,
                             __attribute__((unused)) struct ROGUE_TA_REGION_CLIP0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->mode = __pvr_uint_unpack(dw[0], 31, 31);
    values->left = __pvr_uint_unpack(dw[0], 16, 24);
    values->right = __pvr_uint_unpack(dw[0], 0, 8);
}

#define ROGUE_TA_REGION_CLIP1_length           1
#define ROGUE_TA_REGION_CLIP1_header            \


struct ROGUE_TA_REGION_CLIP1 {
    uint32_t                             top;
    uint32_t                             bottom;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_REGION_CLIP1_pack(__attribute__((unused)) void * restrict dst,
                           __attribute__((unused)) const struct ROGUE_TA_REGION_CLIP1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->top, 16, 24) |
      __pvr_uint(values->bottom, 0, 8);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_REGION_CLIP1_unpack(__attribute__((unused)) const void * restrict src,
                             __attribute__((unused)) struct ROGUE_TA_REGION_CLIP1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->top = __pvr_uint_unpack(dw[0], 16, 24);
    values->bottom = __pvr_uint_unpack(dw[0], 0, 8);
}

#define ROGUE_TA_WCLAMP_length                 1
#define ROGUE_TA_WCLAMP_header                  \


struct ROGUE_TA_WCLAMP {
    uint32_t                             val;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_WCLAMP_pack(__attribute__((unused)) void * restrict dst,
                     __attribute__((unused)) const struct ROGUE_TA_WCLAMP * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->val, 0, 31);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_WCLAMP_unpack(__attribute__((unused)) const void * restrict src,
                       __attribute__((unused)) struct ROGUE_TA_WCLAMP * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->val = __pvr_uint_unpack(dw[0], 0, 31);
}

#define ROGUE_TA_STATE_ISPDBSC_length          1
#define ROGUE_TA_STATE_ISPDBSC_header           \


struct ROGUE_TA_STATE_ISPDBSC {
    uint32_t                             dbindex;
    uint32_t                             scindex;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_ISPDBSC_pack(__attribute__((unused)) void * restrict dst,
                            __attribute__((unused)) const struct ROGUE_TA_STATE_ISPDBSC * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->dbindex, 16, 31) |
      __pvr_uint(values->scindex, 0, 15);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_ISPDBSC_unpack(__attribute__((unused)) const void * restrict src,
                              __attribute__((unused)) struct ROGUE_TA_STATE_ISPDBSC * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->dbindex = __pvr_uint_unpack(dw[0], 16, 31);
    values->scindex = __pvr_uint_unpack(dw[0], 0, 15);
}

#define ROGUE_TA_OUTPUT_SEL_length             1
#define ROGUE_TA_OUTPUT_SEL_header              \


struct ROGUE_TA_OUTPUT_SEL {
    uint32_t                             vtxsize;
    bool                                 tsp_unclamped_z_pres;
    bool                                 render_tgt_pres;
    bool                                 vpt_tgt_pres;
    bool                                 psprite_size_pres;
    bool                                 isp_position_depth_clamp_z;
    bool                                 rhw_pres;
    bool                                 cullplane7;
    bool                                 cullplane6;
    bool                                 cullplane5;
    bool                                 cullplane4;
    bool                                 cullplane3;
    bool                                 cullplane2;
    bool                                 cullplane1;
    bool                                 cullplane0;
    bool                                 plane7;
    bool                                 plane6;
    bool                                 plane5;
    bool                                 plane4;
    bool                                 plane3;
    bool                                 plane2;
    bool                                 plane1;
    bool                                 plane0;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_OUTPUT_SEL_pack(__attribute__((unused)) void * restrict dst,
                         __attribute__((unused)) const struct ROGUE_TA_OUTPUT_SEL * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->vtxsize, 24, 31) |
      __pvr_uint(values->tsp_unclamped_z_pres, 21, 21) |
      __pvr_uint(values->render_tgt_pres, 20, 20) |
      __pvr_uint(values->vpt_tgt_pres, 19, 19) |
      __pvr_uint(values->psprite_size_pres, 18, 18) |
      __pvr_uint(values->isp_position_depth_clamp_z, 17, 17) |
      __pvr_uint(values->rhw_pres, 16, 16) |
      __pvr_uint(values->cullplane7, 15, 15) |
      __pvr_uint(values->cullplane6, 14, 14) |
      __pvr_uint(values->cullplane5, 13, 13) |
      __pvr_uint(values->cullplane4, 12, 12) |
      __pvr_uint(values->cullplane3, 11, 11) |
      __pvr_uint(values->cullplane2, 10, 10) |
      __pvr_uint(values->cullplane1, 9, 9) |
      __pvr_uint(values->cullplane0, 8, 8) |
      __pvr_uint(values->plane7, 7, 7) |
      __pvr_uint(values->plane6, 6, 6) |
      __pvr_uint(values->plane5, 5, 5) |
      __pvr_uint(values->plane4, 4, 4) |
      __pvr_uint(values->plane3, 3, 3) |
      __pvr_uint(values->plane2, 2, 2) |
      __pvr_uint(values->plane1, 1, 1) |
      __pvr_uint(values->plane0, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_OUTPUT_SEL_unpack(__attribute__((unused)) const void * restrict src,
                           __attribute__((unused)) struct ROGUE_TA_OUTPUT_SEL * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->vtxsize = __pvr_uint_unpack(dw[0], 24, 31);
    values->tsp_unclamped_z_pres = __pvr_uint_unpack(dw[0], 21, 21);
    values->render_tgt_pres = __pvr_uint_unpack(dw[0], 20, 20);
    values->vpt_tgt_pres = __pvr_uint_unpack(dw[0], 19, 19);
    values->psprite_size_pres = __pvr_uint_unpack(dw[0], 18, 18);
    values->isp_position_depth_clamp_z = __pvr_uint_unpack(dw[0], 17, 17);
    values->rhw_pres = __pvr_uint_unpack(dw[0], 16, 16);
    values->cullplane7 = __pvr_uint_unpack(dw[0], 15, 15);
    values->cullplane6 = __pvr_uint_unpack(dw[0], 14, 14);
    values->cullplane5 = __pvr_uint_unpack(dw[0], 13, 13);
    values->cullplane4 = __pvr_uint_unpack(dw[0], 12, 12);
    values->cullplane3 = __pvr_uint_unpack(dw[0], 11, 11);
    values->cullplane2 = __pvr_uint_unpack(dw[0], 10, 10);
    values->cullplane1 = __pvr_uint_unpack(dw[0], 9, 9);
    values->cullplane0 = __pvr_uint_unpack(dw[0], 8, 8);
    values->plane7 = __pvr_uint_unpack(dw[0], 7, 7);
    values->plane6 = __pvr_uint_unpack(dw[0], 6, 6);
    values->plane5 = __pvr_uint_unpack(dw[0], 5, 5);
    values->plane4 = __pvr_uint_unpack(dw[0], 4, 4);
    values->plane3 = __pvr_uint_unpack(dw[0], 3, 3);
    values->plane2 = __pvr_uint_unpack(dw[0], 2, 2);
    values->plane1 = __pvr_uint_unpack(dw[0], 1, 1);
    values->plane0 = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_TA_STATE_VARYING0_length         1
#define ROGUE_TA_STATE_VARYING0_header          \


struct ROGUE_TA_STATE_VARYING0 {
    uint32_t                             f32_npc;
    uint32_t                             f32_flat;
    uint32_t                             f32_linear;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_VARYING0_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_TA_STATE_VARYING0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->f32_npc, 16, 23) |
      __pvr_uint(values->f32_flat, 8, 15) |
      __pvr_uint(values->f32_linear, 0, 7);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_VARYING0_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_TA_STATE_VARYING0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->f32_npc = __pvr_uint_unpack(dw[0], 16, 23);
    values->f32_flat = __pvr_uint_unpack(dw[0], 8, 15);
    values->f32_linear = __pvr_uint_unpack(dw[0], 0, 7);
}

#define ROGUE_TA_STATE_VARYING1_length         1
#define ROGUE_TA_STATE_VARYING1_header          \


struct ROGUE_TA_STATE_VARYING1 {
    uint32_t                             f16_npc;
    uint32_t                             f16_flat;
    uint32_t                             f16_linear;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_VARYING1_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_TA_STATE_VARYING1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->f16_npc, 24, 31) |
      __pvr_uint(values->f16_flat, 16, 23) |
      __pvr_uint(values->f16_linear, 8, 15);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_VARYING1_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_TA_STATE_VARYING1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->f16_npc = __pvr_uint_unpack(dw[0], 24, 31);
    values->f16_flat = __pvr_uint_unpack(dw[0], 16, 23);
    values->f16_linear = __pvr_uint_unpack(dw[0], 8, 15);
}

#define ROGUE_TA_STATE_VARYING2_length         1
#define ROGUE_TA_STATE_VARYING2_header          \


struct ROGUE_TA_STATE_VARYING2 {
    uint32_t                             output_clip_planes;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_VARYING2_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_TA_STATE_VARYING2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->output_clip_planes, 16, 19);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_VARYING2_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_TA_STATE_VARYING2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->output_clip_planes = __pvr_uint_unpack(dw[0], 16, 19);
}

#define ROGUE_TA_STATE_TERMINATE0_length       1
#define ROGUE_TA_STATE_TERMINATE0_header        \


/* Helper macros for clip_right */
#define ROGUE_TA_STATE_TERMINATE0_CLIP_RIGHT_BLOCK_SIZE_IN_PIXELS 32

/* Helper macros for clip_top */
#define ROGUE_TA_STATE_TERMINATE0_CLIP_TOP_BLOCK_SIZE_IN_PIXELS 32

/* Helper macros for clip_bottom */
#define ROGUE_TA_STATE_TERMINATE0_CLIP_BOTTOM_BLOCK_SIZE_IN_PIXELS 32

struct ROGUE_TA_STATE_TERMINATE0 {
    uint32_t                             clip_right;
    uint32_t                             clip_top;
    uint32_t                             clip_bottom;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_TERMINATE0_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_TA_STATE_TERMINATE0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->clip_right, 18, 26) |
      __pvr_uint(values->clip_top, 9, 17) |
      __pvr_uint(values->clip_bottom, 0, 8);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_TERMINATE0_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_TA_STATE_TERMINATE0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->clip_right = __pvr_uint_unpack(dw[0], 18, 26);
    values->clip_top = __pvr_uint_unpack(dw[0], 9, 17);
    values->clip_bottom = __pvr_uint_unpack(dw[0], 0, 8);
}

#define ROGUE_TA_STATE_TERMINATE1_length       1
#define ROGUE_TA_STATE_TERMINATE1_header        \


/* Helper macros for clip_left */
#define ROGUE_TA_STATE_TERMINATE1_CLIP_LEFT_BLOCK_SIZE_IN_PIXELS 32

struct ROGUE_TA_STATE_TERMINATE1 {
    uint32_t                             clip_left;
    uint32_t                             render_target;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_TERMINATE1_pack(__attribute__((unused)) void * restrict dst,
                               __attribute__((unused)) const struct ROGUE_TA_STATE_TERMINATE1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->clip_left, 23, 31) |
      __pvr_uint(values->render_target, 0, 10);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_TERMINATE1_unpack(__attribute__((unused)) const void * restrict src,
                                 __attribute__((unused)) struct ROGUE_TA_STATE_TERMINATE1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->clip_left = __pvr_uint_unpack(dw[0], 23, 31);
    values->render_target = __pvr_uint_unpack(dw[0], 0, 10);
}

#define ROGUE_TA_STATE_STREAM_OUT0_length      1
#define ROGUE_TA_STATE_STREAM_OUT0_header       \


struct ROGUE_TA_STATE_STREAM_OUT0 {
    uint32_t                             stream3_size;
    uint32_t                             stream2_size;
    uint32_t                             stream1_size;
    bool                                 stream0_mem_output;
    bool                                 stream0_ta_output;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_STREAM_OUT0_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_TA_STATE_STREAM_OUT0 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->stream3_size, 24, 31) |
      __pvr_uint(values->stream2_size, 16, 23) |
      __pvr_uint(values->stream1_size, 8, 15) |
      __pvr_uint(values->stream0_mem_output, 1, 1) |
      __pvr_uint(values->stream0_ta_output, 0, 0);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_STREAM_OUT0_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_TA_STATE_STREAM_OUT0 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->stream3_size = __pvr_uint_unpack(dw[0], 24, 31);
    values->stream2_size = __pvr_uint_unpack(dw[0], 16, 23);
    values->stream1_size = __pvr_uint_unpack(dw[0], 8, 15);
    values->stream0_mem_output = __pvr_uint_unpack(dw[0], 1, 1);
    values->stream0_ta_output = __pvr_uint_unpack(dw[0], 0, 0);
}

#define ROGUE_TA_STATE_STREAM_OUT1_length      1
#define ROGUE_TA_STATE_STREAM_OUT1_header       \


/* Helper macros for pds_data_size */
#define ROGUE_TA_STATE_STREAM_OUT1_PDS_DATA_SIZE_UNIT_SIZE 16

/* Helper macros for pds_temp_size */
#define ROGUE_TA_STATE_STREAM_OUT1_PDS_TEMP_SIZE_UNIT_SIZE 16

struct ROGUE_TA_STATE_STREAM_OUT1 {
    bool                                 sync;
    uint32_t                             pds_data_size;
    uint32_t                             pds_temp_size;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_STREAM_OUT1_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_TA_STATE_STREAM_OUT1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->sync, 10, 10) |
      __pvr_uint(values->pds_data_size, 4, 9) |
      __pvr_uint(values->pds_temp_size, 0, 3);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_STREAM_OUT1_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_TA_STATE_STREAM_OUT1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->sync = __pvr_uint_unpack(dw[0], 10, 10);
    values->pds_data_size = __pvr_uint_unpack(dw[0], 4, 9);
    values->pds_temp_size = __pvr_uint_unpack(dw[0], 0, 3);
}

#define ROGUE_TA_STATE_STREAM_OUT2_length      1
#define ROGUE_TA_STATE_STREAM_OUT2_header       \


/* Helper macros for pds_data_addr */
#define ROGUE_TA_STATE_STREAM_OUT2_PDS_DATA_ADDR_ALIGNMENT 16

struct ROGUE_TA_STATE_STREAM_OUT2 {
    __pvr_address_type                   pds_data_addr;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_STREAM_OUT2_pack(__attribute__((unused)) void * restrict dst,
                                __attribute__((unused)) const struct ROGUE_TA_STATE_STREAM_OUT2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->pds_data_addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_STREAM_OUT2_unpack(__attribute__((unused)) const void * restrict src,
                                  __attribute__((unused)) struct ROGUE_TA_STATE_STREAM_OUT2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_data_addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_TA_STATE_PDS_SHADERBASE_length      1
#define ROGUE_TA_STATE_PDS_SHADERBASE_header    \


/* Helper macros for addr */
#define ROGUE_TA_STATE_PDS_SHADERBASE_ADDR_ALIGNMENT 16

struct ROGUE_TA_STATE_PDS_SHADERBASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_SHADERBASE_pack(__attribute__((unused)) void * restrict dst,
                                   __attribute__((unused)) const struct ROGUE_TA_STATE_PDS_SHADERBASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_SHADERBASE_unpack(__attribute__((unused)) const void * restrict src,
                                     __attribute__((unused)) struct ROGUE_TA_STATE_PDS_SHADERBASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_TA_STATE_PDS_TEXUNICODEBASE_length      1
#define ROGUE_TA_STATE_PDS_TEXUNICODEBASE_header\


/* Helper macros for addr */
#define ROGUE_TA_STATE_PDS_TEXUNICODEBASE_ADDR_ALIGNMENT 16

struct ROGUE_TA_STATE_PDS_TEXUNICODEBASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_TEXUNICODEBASE_pack(__attribute__((unused)) void * restrict dst,
                                       __attribute__((unused)) const struct ROGUE_TA_STATE_PDS_TEXUNICODEBASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_TEXUNICODEBASE_unpack(__attribute__((unused)) const void * restrict src,
                                         __attribute__((unused)) struct ROGUE_TA_STATE_PDS_TEXUNICODEBASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_TA_STATE_PDS_VARYINGBASE_length      1
#define ROGUE_TA_STATE_PDS_VARYINGBASE_header   \


/* Helper macros for addr */
#define ROGUE_TA_STATE_PDS_VARYINGBASE_ADDR_ALIGNMENT 16

struct ROGUE_TA_STATE_PDS_VARYINGBASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_VARYINGBASE_pack(__attribute__((unused)) void * restrict dst,
                                    __attribute__((unused)) const struct ROGUE_TA_STATE_PDS_VARYINGBASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_VARYINGBASE_unpack(__attribute__((unused)) const void * restrict src,
                                      __attribute__((unused)) struct ROGUE_TA_STATE_PDS_VARYINGBASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_TA_STATE_PDS_TEXTUREDATABASE_length      1
#define ROGUE_TA_STATE_PDS_TEXTUREDATABASE_header\


/* Helper macros for addr */
#define ROGUE_TA_STATE_PDS_TEXTUREDATABASE_ADDR_ALIGNMENT 16

struct ROGUE_TA_STATE_PDS_TEXTUREDATABASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_TEXTUREDATABASE_pack(__attribute__((unused)) void * restrict dst,
                                        __attribute__((unused)) const struct ROGUE_TA_STATE_PDS_TEXTUREDATABASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_TEXTUREDATABASE_unpack(__attribute__((unused)) const void * restrict src,
                                          __attribute__((unused)) struct ROGUE_TA_STATE_PDS_TEXTUREDATABASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_TA_STATE_PDS_UNIFORMDATABASE_length      1
#define ROGUE_TA_STATE_PDS_UNIFORMDATABASE_header\


/* Helper macros for addr */
#define ROGUE_TA_STATE_PDS_UNIFORMDATABASE_ADDR_ALIGNMENT 16

struct ROGUE_TA_STATE_PDS_UNIFORMDATABASE {
    __pvr_address_type                   addr;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_UNIFORMDATABASE_pack(__attribute__((unused)) void * restrict dst,
                                        __attribute__((unused)) const struct ROGUE_TA_STATE_PDS_UNIFORMDATABASE * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] = __pvr_address(values->addr, 4, 4, 31) | 0;
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_UNIFORMDATABASE_unpack(__attribute__((unused)) const void * restrict src,
                                          __attribute__((unused)) struct ROGUE_TA_STATE_PDS_UNIFORMDATABASE * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->addr = __pvr_address_unpack(dw[0], 4, 4, 31);
}

#define ROGUE_TA_STATE_PDS_SIZEINFO1_length      1
#define ROGUE_TA_STATE_PDS_SIZEINFO1_header     \


/* Helper macros for pds_uniformsize */
#define ROGUE_TA_STATE_PDS_SIZEINFO1_PDS_UNIFORMSIZE_UNIT_SIZE 4

/* Helper macros for pds_texturestatesize */
#define ROGUE_TA_STATE_PDS_SIZEINFO1_PDS_TEXTURESTATESIZE_UNIT_SIZE 4

/* Helper macros for pds_varyingsize */
#define ROGUE_TA_STATE_PDS_SIZEINFO1_PDS_VARYINGSIZE_UNIT_SIZE 4

/* Helper macros for usc_varyingsize */
#define ROGUE_TA_STATE_PDS_SIZEINFO1_USC_VARYINGSIZE_UNIT_SIZE 16
#define ROGUE_TA_STATE_PDS_SIZEINFO1_USC_VARYINGSIZE_MAX_SIZE 63

/* Helper macros for pds_tempsize */
#define ROGUE_TA_STATE_PDS_SIZEINFO1_PDS_TEMPSIZE_UNIT_SIZE 4

struct ROGUE_TA_STATE_PDS_SIZEINFO1 {
    uint32_t                             pds_uniformsize;
    uint32_t                             pds_texturestatesize;
    uint32_t                             pds_varyingsize;
    uint32_t                             usc_varyingsize;
    uint32_t                             pds_tempsize;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_SIZEINFO1_pack(__attribute__((unused)) void * restrict dst,
                                  __attribute__((unused)) const struct ROGUE_TA_STATE_PDS_SIZEINFO1 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->pds_uniformsize, 23, 31) |
      __pvr_uint(values->pds_texturestatesize, 16, 22) |
      __pvr_uint(values->pds_varyingsize, 10, 15) |
      __pvr_uint(values->usc_varyingsize, 4, 9) |
      __pvr_uint(values->pds_tempsize, 0, 3);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_SIZEINFO1_unpack(__attribute__((unused)) const void * restrict src,
                                    __attribute__((unused)) struct ROGUE_TA_STATE_PDS_SIZEINFO1 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->pds_uniformsize = __pvr_uint_unpack(dw[0], 23, 31);
    values->pds_texturestatesize = __pvr_uint_unpack(dw[0], 16, 22);
    values->pds_varyingsize = __pvr_uint_unpack(dw[0], 10, 15);
    values->usc_varyingsize = __pvr_uint_unpack(dw[0], 4, 9);
    values->pds_tempsize = __pvr_uint_unpack(dw[0], 0, 3);
}

#define ROGUE_TA_STATE_PDS_SIZEINFO2_length      1
#define ROGUE_TA_STATE_PDS_SIZEINFO2_header     \


/* Helper macros for usc_sharedsize */
#define ROGUE_TA_STATE_PDS_SIZEINFO2_USC_SHAREDSIZE_UNIT_SIZE 16

struct ROGUE_TA_STATE_PDS_SIZEINFO2 {
    uint32_t                             usc_sharedsize;
    bool                                 pds_tri_merge_disable;
    uint32_t                             pds_batchnum;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_SIZEINFO2_pack(__attribute__((unused)) void * restrict dst,
                                  __attribute__((unused)) const struct ROGUE_TA_STATE_PDS_SIZEINFO2 * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->usc_sharedsize, 23, 31) |
      __pvr_uint(values->pds_tri_merge_disable, 14, 14) |
      __pvr_uint(values->pds_batchnum, 0, 13);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PDS_SIZEINFO2_unpack(__attribute__((unused)) const void * restrict src,
                                    __attribute__((unused)) struct ROGUE_TA_STATE_PDS_SIZEINFO2 * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->usc_sharedsize = __pvr_uint_unpack(dw[0], 23, 31);
    values->pds_tri_merge_disable = __pvr_uint_unpack(dw[0], 14, 14);
    values->pds_batchnum = __pvr_uint_unpack(dw[0], 0, 13);
}

#define ROGUE_TA_STATE_PPP_CTRL_length         1
#define ROGUE_TA_STATE_PPP_CTRL_header          \


struct ROGUE_TA_STATE_PPP_CTRL {
    bool                                 prim_msaa;
    enum ROGUE_TA_GS_OUTPUT_TOPOLOGY     gs_output_topology;
    bool                                 pres_prim_id;
    enum ROGUE_TA_CLIP_MODE              clip_mode;
    bool                                 drawclippededges;
    enum ROGUE_TA_FLATSHADE              flatshade_vtx;
    bool                                 pretransform;
    bool                                 wclampen;
    bool                                 wbuffen;
    bool                                 resetbbox;
    bool                                 updatebbox;
    enum ROGUE_TA_CULLMODE               cullmode;
};

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PPP_CTRL_pack(__attribute__((unused)) void * restrict dst,
                             __attribute__((unused)) const struct ROGUE_TA_STATE_PPP_CTRL * restrict values)
{
    uint32_t * restrict dw = (uint32_t * restrict) dst;

    dw[0] =
      __pvr_uint(values->prim_msaa, 15, 15) |
      __pvr_uint(values->gs_output_topology, 13, 14) |
      __pvr_uint(values->pres_prim_id, 12, 12) |
      __pvr_uint(values->clip_mode, 10, 11) |
      __pvr_uint(values->drawclippededges, 9, 9) |
      __pvr_uint(values->flatshade_vtx, 7, 8) |
      __pvr_uint(values->pretransform, 6, 6) |
      __pvr_uint(values->wclampen, 5, 5) |
      __pvr_uint(values->wbuffen, 4, 4) |
      __pvr_uint(values->resetbbox, 3, 3) |
      __pvr_uint(values->updatebbox, 2, 2) |
      __pvr_uint(values->cullmode, 0, 1);
}

static inline __attribute__((always_inline)) void
ROGUE_TA_STATE_PPP_CTRL_unpack(__attribute__((unused)) const void * restrict src,
                               __attribute__((unused)) struct ROGUE_TA_STATE_PPP_CTRL * restrict values)
{
    const uint32_t * restrict dw = (const uint32_t * restrict) src;
    values->prim_msaa = __pvr_uint_unpack(dw[0], 15, 15);
    values->gs_output_topology = __pvr_uint_unpack(dw[0], 13, 14);
    values->pres_prim_id = __pvr_uint_unpack(dw[0], 12, 12);
    values->clip_mode = __pvr_uint_unpack(dw[0], 10, 11);
    values->drawclippededges = __pvr_uint_unpack(dw[0], 9, 9);
    values->flatshade_vtx = __pvr_uint_unpack(dw[0], 7, 8);
    values->pretransform = __pvr_uint_unpack(dw[0], 6, 6);
    values->wclampen = __pvr_uint_unpack(dw[0], 5, 5);
    values->wbuffen = __pvr_uint_unpack(dw[0], 4, 4);
    values->resetbbox = __pvr_uint_unpack(dw[0], 3, 3);
    values->updatebbox = __pvr_uint_unpack(dw[0], 2, 2);
    values->cullmode = __pvr_uint_unpack(dw[0], 0, 1);
}

#endif /* PPP_H */
