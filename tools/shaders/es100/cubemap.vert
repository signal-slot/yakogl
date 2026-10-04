// gl_test case 113 "cubemap": textureCubeLod at a direction and level the
// case aims at one texel of one face, the fetched texel passed through (the
// GC7000's texldl; textureCubeLod is vertex only in GLSL ES 1.00).
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
