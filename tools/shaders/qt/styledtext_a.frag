#version 100
precision highp float;
precision highp int;

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

uniform buf _40;

uniform highp sampler2D _qt_texture;

varying vec2 sampleCoord;
varying vec2 shiftedSampleCoord;

void main()
{
    float glyph = texture2D(_qt_texture, sampleCoord).w;
    float style = clamp(texture2D(_qt_texture, shiftedSampleCoord).w - glyph, 0.0, 1.0);
    gl_FragData[0] = (_40.styleColor * style) + (_40.color * glyph);
}

