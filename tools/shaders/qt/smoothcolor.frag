#version 100
precision highp float;
precision highp int;

varying vec4 color;

void main()
{
    gl_FragData[0] = color;
}

