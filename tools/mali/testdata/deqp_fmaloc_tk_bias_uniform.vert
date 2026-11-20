attribute highp vec4 a_position;
attribute mediump vec2 a_texCoord;
varying mediump vec2 v_texCoord;

void main (void)
{
    gl_Position = a_position;
    v_texCoord = a_texCoord;
}
