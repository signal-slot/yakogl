#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: three mediump vec3 varyings packed across three slots (bump fs 77c5's NormalEye, TangentEye, BitangentEye; terrain fs 4d26's vTangent, vBinormal, vNormal)
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
attribute vec4 a_pos;
attribute vec3 a_n;
attribute vec3 a_t;
attribute vec3 a_b;
varying vec3 v_n;
varying vec3 v_t;
varying vec3 v_b;
void main() { gl_Position = a_pos; v_n = a_n; v_t = a_t; v_b = a_b; }
