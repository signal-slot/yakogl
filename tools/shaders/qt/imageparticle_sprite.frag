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

varying vec4 fTexS;
varying vec2 tt;
varying vec4 fColor;

void main()
{
    gl_FragData[0] = ((mix(texture2D(_qt_texture, fTexS.xy), texture2D(_qt_texture, fTexS.zw), vec4(tt.y)) * fColor) * texture2D(colortable, tt)) * ubuf.opacity;
}

