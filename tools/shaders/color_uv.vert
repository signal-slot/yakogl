#version 450
/* Two-varying VS: colour + uv (matches color_uv.frag). */
layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec4 a_color;
layout(location = 2) in vec2 a_uv;
layout(location = 0) out vec4 v_color;
layout(location = 1) out vec2 v_uv;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
    v_uv = a_uv;
}
