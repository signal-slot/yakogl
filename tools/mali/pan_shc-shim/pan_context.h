/* SPDX-License-Identifier: MIT
 *
 * pan_shc's stand-in for the gallium panfrost driver's pan_context.h.
 *
 * build_pan_shc.sh copies three passes of the gallium driver, unmodified, next
 * to pan_shc.c (pan_nir_lower_sysvals.c, pan_nir_lower_res_indices.c,
 * pan_nir_remove_fragcolor_stores.c). Their `#include "pan_context.h"` then
 * finds this file instead of the driver's, which pulls in the whole pipe
 * context. Everything below is transcribed from Mesa 26.2.1 (MIT, Copyright
 * Collabora, Ltd. and others), src/gallium/drivers/panfrost/pan_context.h
 * lines 277-345 (the system value infrastructure and the Valhall resource
 * tables), and the prototypes of the three passes from the same header.
 */
#ifndef PAN_SHC_PAN_CONTEXT_H
#define PAN_SHC_PAN_CONTEXT_H

#include "compiler/nir/nir.h"
#include "util/hash_table.h"
#include "pan_compiler.h"

/* System value infrastructure */
#define MAX_SYSVAL_COUNT 32

/* Allow 2D of sysval IDs, while allowing nonparametric sysvals to equal
 * their class for equal comparison */

#define PAN_SYSVAL(type, no)    (((no) << 16) | PAN_SYSVAL_##type)
#define PAN_SYSVAL_TYPE(sysval) ((sysval)&0xffff)
#define PAN_SYSVAL_ID(sysval)   ((sysval) >> 16)

/* Define some common types. We start at one for easy indexing of hash
 * tables internal to the compiler */

enum {
   PAN_SYSVAL_VIEWPORT_SCALE = 1,
   PAN_SYSVAL_VIEWPORT_OFFSET = 2,
   PAN_SYSVAL_TEXTURE_SIZE = 3,
   PAN_SYSVAL_SSBO = 4,
   PAN_SYSVAL_NUM_WORK_GROUPS = 5,
   PAN_SYSVAL_SAMPLER = 7,
   PAN_SYSVAL_LOCAL_GROUP_SIZE = 8,
   PAN_SYSVAL_WORK_DIM = 9,
   PAN_SYSVAL_IMAGE_SIZE = 10,
   PAN_SYSVAL_SAMPLE_POSITIONS = 11,
   PAN_SYSVAL_MULTISAMPLED = 12,
   PAN_SYSVAL_RT_CONVERSION = 13,
   PAN_SYSVAL_VERTEX_INSTANCE_OFFSETS = 14,
   PAN_SYSVAL_DRAWID = 15,
   PAN_SYSVAL_BLEND_CONSTANTS = 16,
   PAN_SYSVAL_XFB = 17,
   PAN_SYSVAL_NUM_VERTICES = 18,
   PAN_SYSVAL_PRINTF_BUFFER = 19,
   PAN_SYSVAL_IMAGE_SAMPLES = 20,
};

#define PAN_TXS_SYSVAL_ID(texidx, dim, is_array)                               \
   ((texidx) | ((dim) << 7) | ((is_array) ? (1 << 9) : 0))

#define PAN_SYSVAL_ID_TO_TXS_TEX_IDX(id)  ((id)&0x7f)
#define PAN_SYSVAL_ID_TO_TXS_DIM(id)      (((id) >> 7) & 0x3)
#define PAN_SYSVAL_ID_TO_TXS_IS_ARRAY(id) !!((id) & (1 << 9))

/* Sysvals are always mapped to UBO1 */
#define PAN_UBO_SYSVALS 1

struct panfrost_sysvals {
   /* The mapping of sysvals to uniforms, the count, and the off-by-one inverse */
   unsigned sysvals[MAX_SYSVAL_COUNT];
   unsigned sysval_count;
};

/* On Valhall, the driver gives the hardware a table of resource tables.
 * Resources are addressed as the index of the table together with the index of
 * the resource within the table. For simplicity, we put one type of resource
 * in each table and fix the numbering of the tables.
 *
 * This numbering is arbitrary.
 */
enum panfrost_resource_table {
   PAN_TABLE_UBO = 0,
   PAN_TABLE_ATTRIBUTE,
   PAN_TABLE_ATTRIBUTE_BUFFER,
   PAN_TABLE_SAMPLER,
   PAN_TABLE_TEXTURE,
   PAN_TABLE_IMAGE,
   PAN_TABLE_SSBO,

   PAN_NUM_RESOURCE_TABLES
};

bool panfrost_nir_remove_fragcolor_stores(nir_shader *s, unsigned nr_cbufs);

bool panfrost_nir_lower_sysvals(nir_shader *s, unsigned arch,
                                struct panfrost_sysvals *sysvals);

bool panfrost_nir_lower_res_indices(nir_shader *shader, uint64_t gpu_id);

#endif
