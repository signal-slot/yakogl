#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 ref-fau-page: glmark2 terrain vs 18bc's uniform layout without
 * its two vertex look-ups (the displacement read from an attribute lane
 * instead): three mat4 (eye, normal, projection) and a scale and bias float,
 * five varyings (vec4 here, so the fragment shader is one our compiler
 * takes; original reduction)
 * (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1; family
 * FAU_PAGE). */
attribute vec3 a_pos;
attribute vec3 a_nrm;
attribute vec3 a_tan;
attribute vec2 a_uv;
uniform mat4 u_eye;
uniform mat4 u_nrm;
uniform mat4 u_proj;
uniform float u_scale;
uniform float u_bias;
varying vec4 v_tan;
varying vec4 v_bin;
varying vec4 v_nrm;
varying vec4 v_uv;
varying vec4 v_view;
void main() {
    vec4 e = u_eye * vec4(a_pos, 1.0);
    v_view = vec4(-e.xyz, 1.0);
    vec3 n = normalize(vec3(u_nrm * vec4(a_nrm, 1.0)));
    vec3 t = normalize(vec3(u_nrm * vec4(a_tan, 1.0)));
    v_tan = vec4(t, 1.0);
    v_bin = vec4(normalize(cross(n, t)), 1.0);
    v_uv = vec4(a_uv, 0.0, 1.0);
    float d = u_scale * a_uv.x + u_bias;
    gl_Position = u_proj * (vec4(n * d, 0.0) + e);
    v_nrm = vec4(vec3(u_nrm * vec4(a_nrm * 2.0 - 1.0, 1.0)), 1.0);
}
