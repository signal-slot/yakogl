#version 300 es
/* A pass-through for es300/loop_exit.frag. */
in vec4 a_pos;
in vec4 a_color;
out vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
}
