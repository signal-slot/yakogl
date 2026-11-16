#version 100
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;
attribute float a_in0;
varying float v_in0;
attribute float a_in1;
varying float in1;
attribute float a_in2;
varying float v_in2;

void main()
{
    gl_Position = dEQP_Position;
    v_in0 = a_in0;
    in1 = a_in1;
    v_in2 = a_in2;
}
