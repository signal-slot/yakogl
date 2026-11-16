#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 ref-fau-page: glmark2 refract vs 7215's uniform layout, four
 * mat4 (clip, normal, eye and light transforms) of one vec3 position and a
 * vec3 normal, three varyings (vec4 here, so the fragment shader is one
 * our compiler takes; original reduction)
 * (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1; family
 * FAU_PAGE). */
attribute vec3 a_pos;
attribute vec3 a_nrm;
uniform mat4 u_clip;
uniform mat4 u_nrm;
uniform mat4 u_eye;
uniform mat4 u_light;
varying vec4 v_nrm;
varying vec4 v_eye;
varying vec4 v_light;
void main() {
    vec4 p = vec4(a_pos, 1.0);
    v_nrm = vec4(normalize(vec3(u_nrm * vec4(a_nrm, 1.0))), 1.0);
    v_eye = u_eye * p;
    v_light = u_light * p;
    gl_Position = u_clip * p;
}
