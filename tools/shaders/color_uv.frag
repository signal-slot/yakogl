#version 450
/* Two-varying FS: colour modulated by uv. */
layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_uv;
layout(location = 0) out vec4 o_color;
void main() { o_color = v_color * vec4(v_uv, 1.0, 1.0); }
