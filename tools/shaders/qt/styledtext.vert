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
varying vec2 shiftedSampleCoord;
attribute vec4 vCoord;

void main()
{
    sampleCoord = tCoord * _17.textureScale;
    shiftedSampleCoord = (tCoord - _17.shift) * _17.textureScale;
    vec4 xformed = _17.modelViewMatrix * vCoord;
    gl_Position = _17.projectionMatrix * vec4(floor((xformed.xyz * _17.dpr) + vec3(0.5)) / vec3(_17.dpr), xformed.w);
}

