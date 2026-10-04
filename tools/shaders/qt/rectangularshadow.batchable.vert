#version 100

struct buf
{
    mat4 qt_Matrix;
    float qt_Opacity;
    vec4 color;
    vec3 iResolution;
    vec2 rectSize;
    float radius;
    float blur;
};

uniform buf _31;

varying vec2 texCoord;
attribute vec2 qt_MultiTexCoord0;
varying vec2 fragCoord;
attribute vec4 qt_Vertex;
attribute float _qt_order;

void main()
{
    texCoord = qt_MultiTexCoord0;
    fragCoord = qt_Vertex.xy;
    gl_Position = _31.qt_Matrix * qt_Vertex;
    gl_Position.z = _qt_order * gl_Position.w;
}

