#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: two mediump vec3 varyings in one fp16 sum: the link packs v_e.x into slot 0's w beside v_n.xyz (ideas fs c565 and 1aca's vertex_normal and eye_direction)
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
attribute vec4 a_pos;
attribute vec3 a_n;
attribute vec3 a_e;
varying vec3 v_n;
varying vec3 v_e;
void main() { gl_Position = a_pos; v_n = a_n; v_e = a_e; }
