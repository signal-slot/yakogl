#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: four mediump vec3 varyings summed in fp16, v_v.xy in slot 2's lanes 1 and 2 (terrain fs 4d26 shape)
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
precision mediump float;
varying vec3 v_t;
varying vec3 v_b;
varying vec3 v_n;
varying vec3 v_v;
void main() { gl_FragColor = vec4(v_t + v_b + v_n + v_v, 1.0); }
