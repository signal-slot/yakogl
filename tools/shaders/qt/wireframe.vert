#version 100

struct buf
{
    mat4 qt_Matrix;
};

uniform buf ubuf;

varying vec3 barycentric;
attribute vec3 vertexBarycentric;
attribute vec4 vertexCoord;
attribute vec3 normalExt;

void main()
{
    barycentric = vertexBarycentric;
    gl_Position = ubuf.qt_Matrix * vertexCoord;
}

