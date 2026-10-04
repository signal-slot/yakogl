#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 matrix;
    float opacity;
    float entry;
    float timestamp;
    float dpr;
    float sizetable[64];
    float opacitytable[64];
};

uniform buf ubuf;

uniform highp sampler2D _qt_texture;
uniform highp sampler2D colortable;

varying vec2 fTex;
varying vec4 fColor;
varying vec2 tt;

void main()
{
    gl_FragData[0] = ((texture2D(_qt_texture, fTex) * fColor) * texture2D(colortable, tt)) * ubuf.opacity;
}

