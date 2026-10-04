#version 100

struct buf
{
    mat4 matrix;
    vec4 color;
};

uniform buf _19;

attribute vec4 vertexCoord;

void main()
{
    gl_Position = _19.matrix * vertexCoord;
}

