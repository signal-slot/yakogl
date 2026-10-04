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
    float outlineAlphaMax0;
    float outlineAlphaMax1;
};

uniform buf _30;

uniform highp sampler2D _qt_texture;

varying vec2 sampleCoord;

void main()
{
    float _distance = texture2D(_qt_texture, sampleCoord).x;
    float f = fwidth(_distance);
    float outlineLimit = ((_30.outlineAlphaMax1 - _30.outlineAlphaMax0) / 2.0) + _30.outlineAlphaMax0;
    float a = smoothstep(max(0.0, 0.5 - f), min(1.0, 0.5 + f), _distance);
    gl_FragData[0] = (mix(_30.styleColor, _30.color, vec4(a)) * step(1.0 - a, 1.0)) * smoothstep(max(0.0, outlineLimit - f), min(outlineLimit + f, 0.5 - f), _distance);
}

