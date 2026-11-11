#version 300 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// samplerCubeShadow, the plain form: T, R, DREF, TMUSCM; the direction
// (a uniform plus the varying, so nothing folds) is normalised to its major
// axis first, the reference is not.
precision highp float;
uniform highp samplerCubeShadow s;
uniform vec3 dir;
uniform float ref;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = vec4(texture(s, vec4(dir + vec3(v_xy, 0.0), ref)));
}
