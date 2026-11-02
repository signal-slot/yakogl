#version 300 es
in vec4 a_pos;
out vec2 v_xy;
void main() {
    v_xy = a_pos.xy;
    gl_Position = a_pos;
}
