#version 100
/* #32 plan #367 lower-tex-swizzle: vertex shader of the gm2_texswz cases (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_SWIZZLE). */
attribute vec4 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = a_pos;
}
