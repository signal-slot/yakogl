precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute vec2 in0;
attribute vec2 a_in1;
attribute float in2;
attribute float in3;
attribute float a_in4;
attribute float a_in5;
attribute float a_in6;
varying mat3 out0;

void main()
{
    ivec2 in1 = ivec2(a_in1);
int in4 = int(a_in4);
bool in5 = bool(a_in5);
bool in6 = bool(a_in6);

    out0 = mat3(in0, in1, in2, in3, in4, in5, in6);
    gl_Position = dEQP_Position;

}
