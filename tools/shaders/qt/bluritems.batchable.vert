#version 100

struct buf
{
    mat4 qt_Matrix;
    float qt_Opacity;
    vec2 offset;
};

uniform buf _24;

varying vec2 texCoord0;
attribute vec2 qt_MultiTexCoord0;
varying vec2 texCoord1;
varying vec2 texCoord2;
varying vec2 texCoord3;
attribute vec4 qt_Vertex;
attribute float _qt_order;

void main()
{
    float dither = 0.3300000131130218505859375;
    texCoord0 = vec2(qt_MultiTexCoord0.x + _24.offset.x, qt_MultiTexCoord0.y + (_24.offset.y * dither));
    texCoord1 = vec2(qt_MultiTexCoord0.x + (_24.offset.x * dither), qt_MultiTexCoord0.y - _24.offset.y);
    texCoord2 = vec2(qt_MultiTexCoord0.x - (_24.offset.x * dither), qt_MultiTexCoord0.y + _24.offset.y);
    texCoord3 = vec2(qt_MultiTexCoord0.x - _24.offset.x, qt_MultiTexCoord0.y - (_24.offset.y * dither));
    gl_Position = _24.qt_Matrix * qt_Vertex;
    gl_Position.z = _qt_order * gl_Position.w;
}

