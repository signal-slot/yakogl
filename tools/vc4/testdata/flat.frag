#version 450
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// The flat case of tools/v3d/gl_ref.py (FLAT_FS) in the GLSL that
// glslangValidator -G compiles to OpenGL SPIR-V for vc4_shc.
layout(location = 0) uniform vec4 u_color;
layout(location = 0) out vec4 o_color;
void main() { o_color = u_color; }
