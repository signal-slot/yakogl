/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Stand-in for Mesa 26.2.1 src/gallium/drivers/v3d/v3d_context.h (the whole
 * gallium driver: NIR, the pipe context, the screen), which v3dx_format_table.c
 * and tex_check_mesa_fmt.c include and of which they use only what is below.
 */
#ifndef V3D_CONTEXT_STUB_H
#define V3D_CONTEXT_STUB_H

/* The hooks the generated packers want (the gallium driver's v3d_cl.h sets
 * them to its command list and relocations): neither file uses a packer. */
#define __gen_user_data void
#define __gen_address_type uint32_t
#define __gen_address_offset(reloc) (*(reloc))
#define __gen_emit_reloc(cl, reloc)
#define __gen_unpack_address(cl, s, e) ((uint32_t)(__gen_unpack_uint(cl, s, e) << (31 - ((e) - (s)))))

#include <stdint.h>
#include "util/format/u_format.h"
#include "pipe/p_defines.h"
#include "broadcom/common/v3d_device_info.h"
#include "broadcom/common/v3d_macros.h" /* v3dX(), V3DX(): v3d_format_table.h tests v3dX */

#endif
