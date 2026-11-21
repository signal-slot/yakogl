#version 100
/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* rpi5_tex C1: one direction per sample block, the same at all three
 * vertices of its two triangles. */
attribute vec4 a_pos;
attribute vec4 a_dir;
varying vec3 v_dir;
void main() { v_dir = a_dir.xyz; gl_Position = a_pos; }
