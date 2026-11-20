#version 100
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;
attribute mat3 a_in0;
varying mat3 in0;

void main()
{
    gl_Position = dEQP_Position;
    in0 = a_in0;
}
