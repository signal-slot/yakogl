#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// sampler2DArrayShadow at an explicit level: GLSL ES 3.00 8.8 has no
// textureLod of a 2D-array shadow sampler, so textureGrad gives the level
// (uniform gradients): T, I, B, DREF, TMUSLOD, five TMU writes, so Mesa
// compiles the program at 2 threads.
precision highp float;
uniform highp sampler2DArrayShadow s;
uniform float layer;
uniform float ref;
uniform vec2 gx;
uniform vec2 gy;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = vec4(textureGrad(s, vec4(v_xy * 0.5 + 0.5, layer, ref), gx, gy));
}
