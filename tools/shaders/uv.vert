#version 450
/* Position + uv VS (pairs with texture.frag: v_uv at location 0). */
layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec2 a_uv;
layout(location = 0) out vec2 v_uv;
void main() {
    gl_Position = a_pos;
    v_uv = a_uv;
}
