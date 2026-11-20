#version 450
// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// The flat case of tools/v3d/gl_ref.py (FLAT_VS) in the GLSL that
// glslangValidator -G compiles to OpenGL SPIR-V for vc4_shc.
layout(location = 0) in vec4 a_pos;
void main() { gl_Position = a_pos; }
