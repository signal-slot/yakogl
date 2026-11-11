#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// The vertex-stage pins' fragment shader: the vertex colour, unchanged.
precision highp float;
in vec4 v_col;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = v_col;
}
