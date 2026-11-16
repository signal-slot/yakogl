#version 100
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;
attribute vec3 a_in0;
varying vec3 in0;

void main()
{
    gl_Position = dEQP_Position;
    in0 = a_in0;
}
