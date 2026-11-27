#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: the dot product of two mediump vec3 varyings (ideas fs c565 line 44's dot(reflection, eye_direction))
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
precision mediump float;
varying vec3 v_n;
varying vec3 v_e;
void main() { gl_FragColor = vec4(dot(v_n, v_e)); }
