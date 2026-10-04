#version 100

struct buf
{
    mat4 matrix;
    mat4 gradientMatrix;
    vec2 translationPoint;
    vec2 focalToCenter;
    float centerRadius;
    float focalRadius;
    float opacity;
};

uniform buf ubuf;

attribute vec4 vertexCoord;
varying vec2 coord;
attribute float _qt_order;
attribute vec4 vertexColor;

void main()
{
    vec2 gradVertexCoord = (ubuf.gradientMatrix * vertexCoord).xy;
    coord = gradVertexCoord - ubuf.translationPoint;
    gl_Position = ubuf.matrix * vertexCoord;
    gl_Position.z = _qt_order * gl_Position.w;
}

