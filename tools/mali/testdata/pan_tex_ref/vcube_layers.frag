#version 310 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// cube-noseam: vcube.frag as GLSL ES 3.10.
precision mediump float;
layout(location = 0) in mediump vec4 v_color;
layout(location = 0) out mediump vec4 o_color;

void main()
{
    o_color = v_color;
}
