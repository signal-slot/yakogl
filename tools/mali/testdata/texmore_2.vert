#version 100
/* UV-only VS. ES 1.00 twin of gl_test_uv_vs_src. */
attribute vec4 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    gl_Position = a_pos;
    v_uv = a_uv;
}
