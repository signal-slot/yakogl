attribute highp vec4 a_position;
attribute lowp vec4 a_in0;
attribute lowp vec4 a_in1;
varying lowp vec4 v_in0;
varying lowp vec4 v_in1;

void main()
{
    gl_Position = a_position;
    v_in0 = a_in0;
    v_in1 = a_in1;
}
