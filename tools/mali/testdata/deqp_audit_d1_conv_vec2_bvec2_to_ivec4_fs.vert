#version 100
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;
attribute vec2 a_in0;
attribute vec2 a_in1;
varying vec2 in0;
varying vec2 v_in1;

void main()
{
    gl_Position = dEQP_Position;
    in0 = a_in0;
    v_in1 = a_in1;
}
