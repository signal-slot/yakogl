#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: ideas fs c565's three varyings read in one fp16 tree: the vec3s summed beside the whole vec4
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
precision mediump float;
varying vec3 v_n;
varying vec4 v_p;
varying vec3 v_e;
void main() { gl_FragColor = vec4(v_n + v_e, 1.0) + v_p; }
