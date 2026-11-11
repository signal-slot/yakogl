#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// textureGrad on a samplerCube with uniform gradients: the level from the
// gradients projected onto the direction's face.
precision highp float;
uniform highp samplerCube s;
uniform vec3 dir;
uniform vec3 gx;
uniform vec3 gy;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = textureGrad(s, dir + vec3(v_xy, 0.0), gx, gy);
}
