#version 100

struct buf
{
    mat4 qt_Matrix;
    mat4 fillMatrix;
    vec2 boundsSize;
    float qt_Opacity;
};

uniform buf ubuf;

attribute vec4 vertexCoord;
varying vec2 textureCoord;
attribute vec4 vertexColor;

void main()
{
    vec2 xformed = (ubuf.fillMatrix * vertexCoord).xy;
    textureCoord = vec2(xformed.x / ubuf.boundsSize.x, xformed.y / ubuf.boundsSize.y);
    gl_Position = ubuf.qt_Matrix * vertexCoord;
}

