/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Stand-in for Mesa 26.2.1 src/gallium/drivers/v3d/v3dx_context.h (see
 * v3d_context.h here): the one prototype v3dx_format_table.c defines and
 * tex_check_mesa_fmt.c calls.
 */
#include "util/format/u_format.h"
#include "broadcom/common/v3d_macros.h"

struct v3d_format;
const struct v3d_format *v3dX(get_format_desc)(enum pipe_format f);
