#version 100
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

uniform buf _14;

uniform highp sampler2D _qt_texture;

varying vec2 sampleCoord;
varying vec2 shiftedSampleCoord;

void main()
{
    float a = smoothstep(_14.alphaMin, _14.alphaMax, texture2D(_qt_texture, sampleCoord).x);
    vec4 shifted = _14.styleColor * smoothstep(_14.alphaMin, _14.alphaMax, texture2D(_qt_texture, shiftedSampleCoord).x);
    gl_FragData[0] = mix(shifted, _14.color, vec4(a));
}

