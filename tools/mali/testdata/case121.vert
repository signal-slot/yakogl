#version 100
/* gl_test case 121 (uniidxh): gl_test_passthrough_vs_src
 * (driver/tools/pvrlab/gl_test_shaders_es100.h) verbatim, the vertex shader
 * of case 122 as well (uniidx_i.vert); the pair's fragment shader is
 * uniidx_h.frag. */
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
}
