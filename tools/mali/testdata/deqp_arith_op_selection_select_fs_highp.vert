attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
attribute highp vec4 a_in1;
attribute highp vec4 a_in2;
varying mediump vec4 v_in0;
varying highp vec4 v_in1;
varying highp vec4 v_in2;

void main()
{
    gl_Position = a_position;
    v_in0 = a_in0;
    v_in1 = a_in1;
    v_in2 = a_in2;
}
