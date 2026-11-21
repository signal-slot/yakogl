#version 100
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;
attribute vec3 a_in0;
attribute float a_in1;
varying vec3 in0;
varying float in1;

void main()
{
    gl_Position = dEQP_Position;
    in0 = a_in0;
    in1 = a_in1;
}
