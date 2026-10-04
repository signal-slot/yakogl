#version 100

struct buf
{
    mat4 matrix;
};

uniform buf ubuf;

attribute vec4 vCoord;

void main()
{
    gl_Position = ubuf.matrix * vCoord;
}

