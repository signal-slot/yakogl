attribute highp vec4 a_position;
attribute highp vec3 a_texCoord;
varying highp vec3 v_texCoord;

void main (void)
{
    gl_Position = a_position;
    v_texCoord = a_texCoord;
}
