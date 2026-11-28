#version 310 es
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// cube-noseam (docs/notes/mali-cube-noseam.md): the 2D-array look-up the
// cube_noseam variant of vcube.vert ends in, as Mesa 26.2.1 compiles it.
layout(location = 0) in highp vec2 a_position;
layout(location = 1) in highp vec3 a_texCoord;
layout(location = 0) uniform highp float u_lod;
layout(binding = 0) uniform highp sampler2DArray u_texture;
layout(location = 0) out mediump vec4 v_color;

void main()
{
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_color = textureLod(u_texture, a_texCoord, u_lod);
}
