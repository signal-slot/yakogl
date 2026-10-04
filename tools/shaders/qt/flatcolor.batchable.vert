#version 100

struct buf
{
    mat4 matrix;
    vec4 color;
};

uniform buf _19;

attribute vec4 vertexCoord;
attribute float _qt_order;

void main()
{
    gl_Position = _19.matrix * vertexCoord;
    gl_Position.z = _qt_order * gl_Position.w;
}

