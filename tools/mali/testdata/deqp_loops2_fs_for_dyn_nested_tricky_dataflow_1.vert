attribute highp vec4 a_position;
attribute highp vec4 a_coords;
attribute mediump float a_one;
varying mediump vec4 v_coords;
varying mediump float v_one;

void main()
{
    gl_Position = a_position;
    v_coords = a_coords;
    v_one = a_one;
}
