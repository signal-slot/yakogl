#version 100
/* #33 plan #367 lower-const-fold: vertex shader of the gm2_negzero_deep cases (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family NEG_ZERO). */
attribute vec4 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = a_pos;
}
