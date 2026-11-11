#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// The 3D texel offset: textureOffset and textureLodOffset on a sampler3D,
// Offset R in TMU config parameter 2 (bits 16..19).
precision highp float;
uniform highp sampler3D s;
uniform float zz;
uniform float lvl;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    vec3 p = vec3(v_xy * 0.5 + 0.5, zz);
    fragColor = 0.5 * (textureOffset(s, p, ivec3(1, -2, 3)) + textureLodOffset(s, p, lvl, ivec3(-8, 7, -8)));
}
