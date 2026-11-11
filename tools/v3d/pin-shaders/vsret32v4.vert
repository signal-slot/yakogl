#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// A vertex-stage look-up of a 32-bit texture (RGBA32F), all four channels:
// --vs-ret32 0x1, four ldtmu words (P0 0xf).
uniform highp sampler2D s;
layout(location = 0) in vec2 pos;
out vec4 v_col;
void main()
{
    v_col = textureLod(s, pos * 0.5 + 0.5, 0.0);
    gl_Position = vec4(pos, 0.0, 1.0);
}
