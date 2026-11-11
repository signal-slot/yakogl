#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// samplerCubeShadow at an explicit level: GLSL ES 3.00 8.8 has no
// textureLod of a cube shadow sampler, so textureGrad gives the level
// (uniform gradients, so nothing folds): T, R, B, DREF, TMUSCM with P2
// disable_autolod, five TMU writes, past the input FIFO's 16 / 4 slots,
// so Mesa compiles the program at 2 threads (v3d_tex.c v3d_vir_emit_tex).
precision highp float;
uniform highp samplerCubeShadow s;
uniform vec3 dir;
uniform float ref;
uniform vec3 gx;
uniform vec3 gy;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = vec4(textureGrad(s, vec4(dir + vec3(v_xy, 0.0), ref), gx, gy));
}
