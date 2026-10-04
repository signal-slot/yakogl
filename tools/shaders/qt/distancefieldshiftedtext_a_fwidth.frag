#version 100
#extension GL_OES_standard_derivatives : require
precision highp float;
precision highp int;

struct buf
{
    mat4 matrix;
    vec2 textureScale;
    vec4 color;
    float alphaMin;
    float alphaMax;
    vec4 styleColor;
    vec2 shift;
};

uniform buf _55;

uniform highp sampler2D _qt_texture;

varying vec2 sampleCoord;
varying vec2 shiftedSampleCoord;

void main()
{
    float _distance = texture2D(_qt_texture, sampleCoord).w;
    float f = fwidth(_distance);
    float a = smoothstep(0.5 - f, 0.5 + f, _distance);
    float shiftedDistance = texture2D(_qt_texture, shiftedSampleCoord).w;
    float shiftedF = fwidth(shiftedDistance);
    float shiftedA = smoothstep(0.5 - shiftedF, 0.5 + shiftedF, shiftedDistance);
    vec4 shifted = _55.styleColor * shiftedA;
    gl_FragData[0] = mix(shifted, _55.color, vec4(a));
}

