#version 100

struct buf
{
    mat4 modelViewMatrix;
    mat4 projectionMatrix;
    vec2 textureScale;
    float dpr;
    vec4 color;
    vec4 styleColor;
    vec2 shift;
};

uniform buf _17;

varying vec2 sampleCoord;
attribute vec2 tCoord;
varying vec2 sCoordUp;
varying vec2 sCoordDown;
varying vec2 sCoordLeft;
varying vec2 sCoordRight;
attribute vec4 vCoord;
attribute float _qt_order;

void main()
{
    sampleCoord = tCoord * _17.textureScale;
    sCoordUp = (tCoord - vec2(0.0, -1.0)) * _17.textureScale;
    sCoordDown = (tCoord - vec2(0.0, 1.0)) * _17.textureScale;
    sCoordLeft = (tCoord - vec2(-1.0, 0.0)) * _17.textureScale;
    sCoordRight = (tCoord - vec2(1.0, 0.0)) * _17.textureScale;
    vec4 xformed = _17.modelViewMatrix * vCoord;
    gl_Position = _17.projectionMatrix * vec4(floor((xformed.xyz * _17.dpr) + vec3(0.5)) / vec3(_17.dpr), xformed.w);
    gl_Position.z = _qt_order * gl_Position.w;
}

