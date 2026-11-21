attribute highp vec4 a_position;
attribute highp float a_index;
attribute highp vec4 a_color;
varying mediump float v_index;
varying mediump vec4 v_color;
void main (void)
{
    gl_Position = a_position;
    v_color = a_color;
    v_index = a_index;
}
