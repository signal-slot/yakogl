#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// textureGrad on a sampler2D with uniform gradients (so Mesa does not fold
// them): lowered to an explicit level (Mesa's lower_txd; ours,
// docs/notes/v3d-qpu-grad.md).
precision highp float;
uniform highp sampler2D s;
uniform vec2 gx;
uniform vec2 gy;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = textureGrad(s, v_xy * 0.5 + 0.5, gx, gy);
}
