#version 100
/* #1385 (b): gl_test case 122's vertex shader, gl_test_passthrough_vs_src
 * (driver/tools/pvrlab/gl_test_shaders_es100.h) verbatim; the pair's
 * fragment shader is uniidx_i.frag (tools/mali/drm_shim_ref.sh run, Mesa
 * 26.2.1). */
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
}
