#version 300 es
in vec4 a_pos;
out vec4 v_t;
void main() {
    v_t = a_pos;
    gl_Position = a_pos;
}
