/* Vertex shader of fstex_cube (ProgramLibrary, a mediump vec3 coordinate). */
attribute highp vec4 a_position;
attribute mediump vec3 a_texCoord;
varying mediump vec3 v_texCoord;

void main (void)
{
    gl_Position = a_position;
    v_texCoord = a_texCoord;
}
