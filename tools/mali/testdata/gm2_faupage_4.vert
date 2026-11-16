#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 ref-fau-page: one product-sum reading a page-0 word and a
 * page-1 word (u_near is read by the position shader too, so it is pushed on
 * page 0; u_v only by the varying shader, after its two mat4, past word 64;
 * original reduction)
 * (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1; family
 * FAU_PAGE). */
attribute vec4 a_pos;
uniform mat4 u_m0;
uniform mat4 u_m1;
uniform mat4 u_m2;
uniform mat4 u_m3;
uniform float u_near;
uniform vec4 u_v;
varying vec4 v_a;
void main() {
    v_a = (u_m1 * a_pos + u_m2 * a_pos) * u_near + u_v;
    gl_Position = u_m0 * a_pos + u_m3 * a_pos * u_near;
}
