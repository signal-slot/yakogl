#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// sampler2DShadow at an explicit level (TMUB, DREF, TMUSLOD) and with a
// texel offset (P2 Offset S / T, DREF, TMUS).
precision highp float;
uniform highp sampler2DShadow s;
uniform float ref;
uniform float lvl;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    vec3 p = vec3(v_xy * 0.5 + 0.5, ref);
    float a = textureLod(s, p, lvl);
    float b = textureOffset(s, p, ivec2(1, -1));
    fragColor = vec4(a, b, 0.0, 1.0);
}
