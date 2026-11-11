#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// A vertex-stage look-up of a 32-bit texture (R32F), one channel read:
// compiled with --vs-ret32 0x1 (pairs.txt), so Mesa's P1 carries the
// 32-bit output bit and the result is one ldtmu word (P0 0x1).
uniform highp sampler2D s;
layout(location = 0) in vec2 pos;
out vec4 v_col;
void main()
{
    v_col = vec4(textureLod(s, pos * 0.5 + 0.5, 0.0).r, 0.0, 0.0, 1.0);
    gl_Position = vec4(pos, 0.0, 1.0);
}
