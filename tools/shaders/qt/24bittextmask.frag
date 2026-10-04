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
};

uniform buf _26;

uniform highp sampler2D _qt_texture;

varying vec2 sampleCoord;

void main()
{
    vec4 glyph = texture2D(_qt_texture, sampleCoord);
    gl_FragData[0] = glyph * _26.color.w;
}

