#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// textureProj on a sampler2DShadow: s, t and the reference over q (a
// uniform, so nothing folds), then T, DREF, S.
precision highp float;
uniform highp sampler2DShadow s;
uniform float ref;
uniform float q;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = vec4(textureProj(s, vec4(v_xy * 0.5 + 0.5, ref, q)));
}
