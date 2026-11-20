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
attribute vec2 a_in3;
varying vec2 in3;
attribute float a_in4;
varying float v_in4;
attribute vec2 a_in5;
varying vec2 v_in5;
attribute float a_in6;
varying float in6;

void main()
{
    gl_Position = dEQP_Position;
    v_in0 = a_in0;
    in1 = a_in1;
    v_in2 = a_in2;
    in3 = a_in3;
    v_in4 = a_in4;
    v_in5 = a_in5;
    in6 = a_in6;
}
