#version 100
/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * #32 plan #367 ref-fau-page: fragment shader of the gm2_faupage_2 pair
 * (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1). */
precision mediump float;
varying vec4 v_nrm;
varying vec4 v_eye;
varying vec4 v_light;
void main() {
    gl_FragColor = (v_nrm + v_eye) * v_light;
}
