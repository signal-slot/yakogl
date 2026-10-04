#version 100

struct buf
{
    mat4 qt_Matrix;
    float qt_Opacity;
    float hue;
};

uniform buf _25;

varying vec2 qt_TexCoord0;
attribute vec2 qt_VertexTexCoord;
attribute vec4 qt_VertexPosition;
attribute float _qt_order;

void main()
{
    qt_TexCoord0 = qt_VertexTexCoord;
    gl_Position = _25.qt_Matrix * qt_VertexPosition;
    gl_Position.z = _qt_order * gl_Position.w;
}

