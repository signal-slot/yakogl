#version 100
/* #1385 (a): a vertex-stage look-up at the constant level 0.0, the ZERO_LOD
 * mode; tools/shaders/gltest/vtl_vs_grid.vert verbatim
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1, --uniform-values). */
attribute vec2 a_pos;
attribute vec2 a_uv;
uniform sampler2D u_vtex;
varying vec4 v_color;
void main() {
    v_color = texture2DLod(u_vtex, a_uv, 0.0);
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
