/* Varyings (L3 reference): gl_test's color_uv.vert: two copied varyings, the vec4 at byte 0, the vec2 at 16 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
#version 100
/* Two-varying VS: colour + uv. ES 1.00 twin of gl_test_vs_src, the pair most
 * gl_test cases draw with. Order is the twin's: a_pos 0, a_color 1, a_uv 2. */
attribute vec4 a_pos;
attribute vec4 a_color;
attribute vec2 a_uv;
varying vec4 v_color;
varying vec2 v_uv;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
    v_uv = a_uv;
}
