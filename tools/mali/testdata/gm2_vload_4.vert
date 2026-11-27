#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: gm2_vload_1's two vec3 varyings, read by a dot product
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
attribute vec4 a_pos;
attribute vec3 a_n;
attribute vec3 a_e;
varying vec3 v_n;
varying vec3 v_e;
void main() { gl_Position = a_pos; v_n = a_n; v_e = a_e; }
