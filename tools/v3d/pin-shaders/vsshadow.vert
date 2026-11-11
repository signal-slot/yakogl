#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// A vertex-stage shadow look-up: no derivatives, so an implicit texture()
// is level 0 (TMUB), then DREF, retired by TMUSLOD; a depth texture, so
// --vs-ret32 0x1.
uniform highp sampler2DShadow s;
uniform float ref;
layout(location = 0) in vec2 pos;
out vec4 v_col;
void main()
{
    v_col = vec4(texture(s, vec3(pos * 0.5 + 0.5, ref)));
    gl_Position = vec4(pos, 0.0, 1.0);
}
