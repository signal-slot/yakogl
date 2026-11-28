attribute highp vec4 a_position;
attribute highp vec4 a_in0;
varying highp vec4 v_in0;

void main()
{
    gl_Position = a_position;
    v_in0 = a_in0;
}
