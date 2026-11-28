precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float in0;
attribute vec4 a_in1;
attribute vec2 a_in2;
attribute float a_in3;
attribute float a_in4;
varying mat3 out0;

void main()
{
    bvec4 in1 = bvec4(a_in1);
ivec2 in2 = ivec2(a_in2);
bool in3 = bool(a_in3);
bool in4 = bool(a_in4);

    out0 = mat3(in0, in1, in2, in3, in4);
    gl_Position = dEQP_Position;

}
