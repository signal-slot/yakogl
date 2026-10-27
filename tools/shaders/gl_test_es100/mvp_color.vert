#version 100
/* MVP VS with a colour varying. ES 1.00 twin of gl_test_mvp_color_vs_src. */
uniform mat4 u_mvp;
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() {
    gl_Position = u_mvp * a_pos;
    v_color = a_color;
}
