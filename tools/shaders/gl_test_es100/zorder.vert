#version 100
/* Per-vertex depth VS. ES 1.00 twin of gl_test_zorder_vs_src. */
attribute vec4 a_pos;
attribute vec4 a_color;
attribute float a_z;
varying vec4 v_color;
void main() {
    gl_Position = vec4(a_pos.xy, a_z, 1.0);
    v_color = a_color;
}
