/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gles_offline.h - the offline shader table's source texts.
 *
 * On an offline-only target (Zephyr, bare metal) glShaderSource accepts only
 * the GLSL the table in tests/reference/blobs.rs was built from, byte for byte. An
 * application that wants one of those shaders asks for it by its source
 * basename instead of carrying a copy that can drift:
 *
 *   const char *vs = gles_offline_shader_source("passthrough.vert");
 *   glShaderSource(shader, 1, &vs, NULL);
 *
 * Returns NULL for a name that is not in the table.
 */
#ifndef GLES_OFFLINE_H
#define GLES_OFFLINE_H

#ifdef __cplusplus
extern "C" {
#endif

const char *gles_offline_shader_source(const char *name);

#ifdef __cplusplus
}
#endif

#endif /* GLES_OFFLINE_H */
