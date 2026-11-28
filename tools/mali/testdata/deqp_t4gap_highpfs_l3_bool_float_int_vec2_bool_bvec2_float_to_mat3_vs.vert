precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float a_in0;
attribute float in1;
attribute float a_in2;
attribute vec2 in3;
attribute float a_in4;
attribute vec2 a_in5;
attribute float in6;
varying mat3 out0;

void main()
{
    bool in0 = bool(a_in0);
int in2 = int(a_in2);
bool in4 = bool(a_in4);
bvec2 in5 = bvec2(a_in5);

    out0 = mat3(in0, in1, in2, in3, in4, in5, in6);
    gl_Position = dEQP_Position;

}
