#version 100
/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* rpi5_tex T1: the full-screen quad, uv (0,0) at NDC (-1,-1). */
attribute vec4 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() { v_uv = a_uv; gl_Position = a_pos; }
