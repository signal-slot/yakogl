#version 100

struct buf
{
    mat4 matrix;
    mat4 gradientMatrix;
    vec2 translationPoint;
    float angle;
    float opacity;
};

uniform buf ubuf;

attribute vec4 vertexCoord;
varying vec2 coord;
attribute vec4 vertexColor;

void main()
{
    vec2 gradVertexCoord = (ubuf.gradientMatrix * vertexCoord).xy;
    coord = gradVertexCoord - ubuf.translationPoint;
    gl_Position = ubuf.matrix * vertexCoord;
}

