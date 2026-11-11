#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// The pin pairs' vertex shader (tools/shaders/es300/cube.vert's shape): a
// quad whose position is the varying the fragment shaders sample with.
layout(location = 0) in vec2 pos;
out vec2 v_xy;
void main()
{
    v_xy = pos;
    gl_Position = vec4(pos, 0.0, 1.0);
}
