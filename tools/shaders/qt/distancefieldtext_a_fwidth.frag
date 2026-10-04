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
};

uniform buf _31;

uniform highp sampler2D _qt_texture;

varying vec2 sampleCoord;

void main()
{
    float _distance = texture2D(_qt_texture, sampleCoord).w;
    float f = fwidth(_distance);
    gl_FragData[0] = _31.color * smoothstep(max(0.0, 0.5 - f), min(1.0, 0.5 + f), _distance);
}

