#version 100
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;
attribute vec3 a_in0;
varying vec3 in0;
attribute vec3 a_in1;
varying vec3 in1;
attribute vec3 a_in2;
varying vec3 in2;

void main()
{
    gl_Position = dEQP_Position;
    in0 = a_in0;
    in1 = a_in1;
    in2 = a_in2;
}
