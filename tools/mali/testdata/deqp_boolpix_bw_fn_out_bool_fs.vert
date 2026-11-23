#version 100
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;
attribute float a_in0;
varying float v_in0;

void main()
{
    gl_Position = dEQP_Position;
    v_in0 = a_in0;
}
