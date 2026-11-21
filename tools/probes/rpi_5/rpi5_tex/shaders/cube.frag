#version 100
/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* rpi5_tex C1: one cube look-up (gl_test_texcube_fs_src's shape). */
precision mediump float;
uniform samplerCube u_s;
varying vec3 v_dir;
void main() { gl_FragColor = textureCube(u_s, v_dir); }
