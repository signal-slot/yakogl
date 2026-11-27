#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: ideas vs 51b2's layout: a vec3, a vec4, a vec3 (vertex_normal, vertex_position, eye_direction)
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
attribute vec4 a_pos;
attribute vec3 a_n;
attribute vec4 a_p;
attribute vec3 a_e;
varying vec3 v_n;
varying vec4 v_p;
varying vec3 v_e;
void main() { gl_Position = a_pos; v_n = a_n; v_p = a_p; v_e = a_e; }
