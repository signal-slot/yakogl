#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: three mediump vec3 varyings in one fp16 tree, bump fs 77c5 line 46's form
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
precision mediump float;
varying vec3 v_n;
varying vec3 v_t;
varying vec3 v_b;
void main() { gl_FragColor = vec4(v_n - 2.0 * v_t - 3.0 * v_b, 1.0); }
