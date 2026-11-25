attribute highp vec4 a_position;
attribute mediump mat4 a_m0;
varying mediump mat4 v_m0;
attribute mediump mat4 a_m1;
varying mediump mat4 v_m1;

void main (void)
{
    gl_Position = a_position;
    v_m0 = a_m0;
    v_m1 = a_m1;
}
