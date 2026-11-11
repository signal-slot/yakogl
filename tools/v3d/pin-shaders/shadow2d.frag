#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// sampler2DShadow, the plain form: T, DREF, then S retires the look-up, one
// result word (docs/notes/v3d-shadow.md (c)).
precision highp float;
uniform highp sampler2DShadow s;
uniform float ref;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = vec4(texture(s, vec3(v_xy * 0.5 + 0.5, ref)));
}
