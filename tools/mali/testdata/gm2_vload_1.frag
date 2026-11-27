#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: two mediump vec3 varyings in one fp16 sum (ideas fs c565/1aca shape)
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
precision mediump float;
varying vec3 v_n;
varying vec3 v_e;
void main() { gl_FragColor = vec4(v_n + v_e, 1.0); }
