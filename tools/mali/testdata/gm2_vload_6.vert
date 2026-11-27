#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 proto-varying-load: four mediump vec3 varyings over three slots, the fourth from slot 2's odd lane 1 (terrain fs 4d26's vTangent, vBinormal, vNormal, vViewPosition)
 * (tools/mali/deqp_ref.sh --instrument shim, Mesa 26.2.1 on drm-shim; family
 * VARYING_LOAD; original reduction). */
attribute vec4 a_pos;
attribute vec3 a_t;
attribute vec3 a_b;
attribute vec3 a_n;
attribute vec3 a_v;
varying vec3 v_t;
varying vec3 v_b;
varying vec3 v_n;
varying vec3 v_v;
void main() { gl_Position = a_pos; v_t = a_t; v_b = a_b; v_n = a_n; v_v = a_v; }
