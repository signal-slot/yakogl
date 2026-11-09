// gl_test case 101 "cubebias": textureCubeLod at an explicit level, the
// GC7000's texldl (vertex only in GLSL ES 1.00).
attribute vec4 a_pos;
uniform samplerCube u_s;
uniform float u_lod;
varying vec4 v_col;
void main()
{
    gl_Position = a_pos;
    v_col = textureCubeLod(u_s, vec3(1.0, 0.0, 0.0), u_lod);
}
