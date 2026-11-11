#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// A vertex-stage texelFetch of a 32-bit integer texture (RGBA32I):
// --vs-ret32 0x1, the integers as whole words.
uniform highp isampler2D s;
layout(location = 0) in vec2 pos;
out vec4 v_col;
void main()
{
    v_col = vec4(texelFetch(s, ivec2(pos * 4.0 + 4.0), 0));
    gl_Position = vec4(pos, 0.0, 1.0);
}
