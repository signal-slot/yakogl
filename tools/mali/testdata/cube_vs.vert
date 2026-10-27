#version 100
/* #1385 (c): a vertex-stage cube-map look-up at a uniform direction and a
 * uniform level, textureCubeLod's CUBEFACE1/CUBEFACE2/CUBE_SSEL/CUBE_TSEL
 * sequence and TEX_SINGLE.cube EXPLICIT; gl_test case 113's
 * tools/shaders/es100/cubemap.vert verbatim (tools/mali/drm_shim_ref.sh run,
 * Mesa 26.2.1, --uniform-values: u_dir is (32, 33, 34), u_lod 48). */
attribute vec4 a_pos;
uniform samplerCube u_s;
uniform vec3 u_dir;
uniform float u_lod;
varying vec4 v_col;
void main()
{
    gl_Position = a_pos;
    v_col = textureCubeLod(u_s, u_dir, u_lod);
}
