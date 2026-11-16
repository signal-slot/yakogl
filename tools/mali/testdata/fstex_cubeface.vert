/* Vertex shader of fstex_cubeface0-5 (es2fTextureSpecificationTests.cpp
 * TexCubeShader). */
attribute highp vec4 a_position;
attribute mediump vec2 a_coord;
varying mediump vec2 v_coord;
void main (void)
{
    gl_Position = a_position;
    v_coord = a_coord;
}
