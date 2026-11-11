#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// sampler2DArrayShadow, the plain form: T, I (the layer), DREF, TMUS.
precision highp float;
uniform highp sampler2DArrayShadow s;
uniform float layer;
uniform float ref;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = vec4(texture(s, vec4(v_xy * 0.5 + 0.5, layer, ref)));
}
