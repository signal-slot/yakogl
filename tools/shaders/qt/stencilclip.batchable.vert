#version 100

struct buf
{
    mat4 matrix;
};

uniform buf ubuf;

attribute vec4 vCoord;
attribute float _qt_order;

void main()
{
    gl_Position = ubuf.matrix * vCoord;
    gl_Position.z = _qt_order * gl_Position.w;
}

