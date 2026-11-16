#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 ref-fau-page: one uniform word just past FAU page 0 (word 64,
 * the last lane the varying shader reads; original reduction)
 * (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1; family
 * FAU_PAGE). */
attribute vec4 a_pos;
uniform mat4 u_m0;
uniform mat4 u_m1;
uniform mat4 u_m2;
uniform vec4 u_v;
uniform vec2 u_w;
varying vec4 v_a;
void main() {
    v_a = u_m1 * a_pos + u_v * a_pos.y + vec4(u_w, 1.0, 1.0);
    gl_Position = u_m0 * a_pos + u_m2 * a_pos;
}
