#version 100
/* #1385 (a): a vertex-stage look-up at a uniform level, texture2DLod's
 * EXPLICIT mode; gl_test case 197's tools/shaders/gltest/vtxgrid_vs.vert
 * verbatim (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1, --uniform-values). */
attribute highp vec2 a_pos;
attribute highp vec2 a_uv;
uniform highp sampler2D u_vtex;
uniform highp float u_lod;
varying mediump vec4 v_color;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
    v_color = texture2DLod(u_vtex, a_uv, u_lod);
}
