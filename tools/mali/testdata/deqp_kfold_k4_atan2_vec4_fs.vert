#version 100
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;

void main()
{
    gl_Position = dEQP_Position;
}
