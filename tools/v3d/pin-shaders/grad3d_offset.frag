#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// textureGradOffset on a sampler3D with uniform gradients: the level from
// three gradient lanes (the depth scales r) and the 3D texel offset in P2
// (Offset S, T and R).
precision highp float;
uniform highp sampler3D s;
uniform float zz;
uniform vec3 gx;
uniform vec3 gy;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = textureGradOffset(s, vec3(v_xy * 0.5 + 0.5, zz), gx, gy, ivec3(1, -2, 3));
}
