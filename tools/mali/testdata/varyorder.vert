/* gl_test case 209 varyorder (#1386): two vec4 and a vec2, written in this order (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1). */
#version 100
attribute vec4 a_pos;
attribute vec4 a_a;
attribute vec2 a_uv;
attribute vec4 a_b;
varying vec4 v_a;
varying vec2 v_uv;
varying vec4 v_b;
void main() {
    gl_Position = a_pos;
    v_a = a_a;
    v_uv = a_uv;
    v_b = a_b;
}
