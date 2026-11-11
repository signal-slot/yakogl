#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// Two vertex-stage look-ups, one of a 32-bit texture and one of a 16-bit
// one (--vs-ret32 0x1 keys binding 0 alone): the per-sampler mask, two
// look-ups in one stretch, and the 32-bit one's four words ahead of the
// other's two.
uniform highp sampler2D a;
uniform highp sampler2D b;
layout(location = 0) in vec2 pos;
out vec4 v_col;
void main()
{
    vec2 uv = pos * 0.5 + 0.5;
    v_col = textureLod(a, uv, 0.0) + textureLod(b, uv, 1.0);
    gl_Position = vec4(pos, 0.0, 1.0);
}
