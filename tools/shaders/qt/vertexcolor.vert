#version 100

struct buf
{
    mat4 matrix;
    float opacity;
};

uniform buf _19;

attribute vec4 vertexCoord;
varying vec4 color;
attribute vec4 vertexColor;

void main()
{
    gl_Position = _19.matrix * vertexCoord;
    color = vertexColor * _19.opacity;
}

