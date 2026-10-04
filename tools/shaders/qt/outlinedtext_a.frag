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

uniform buf _59;

uniform highp sampler2D _qt_texture;

varying vec2 sampleCoord;
varying vec2 sCoordUp;
varying vec2 sCoordDown;
varying vec2 sCoordLeft;
varying vec2 sCoordRight;

void main()
{
    float glyph = texture2D(_qt_texture, sampleCoord).w;
    float outline = clamp(clamp(((texture2D(_qt_texture, sCoordUp).w + texture2D(_qt_texture, sCoordDown).w) + texture2D(_qt_texture, sCoordLeft).w) + texture2D(_qt_texture, sCoordRight).w, 0.0, 1.0) - glyph, 0.0, 1.0);
    gl_FragData[0] = (_59.styleColor * outline) + (_59.color * (step(1.0 - glyph, 1.0) * glyph));
}

