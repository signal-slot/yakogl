#version 100
/* Point-size VS. ES 1.00 twin of gl_test_points_vs_src. */
attribute vec4 a_pos;
attribute vec4 a_color;
attribute float a_size;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    gl_PointSize = a_size;
    v_color = a_color;
}
