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

varying vec4 fColor;

void main()
{
    gl_FragData[0] = (texture2D(_qt_texture, gl_PointCoord) * fColor) * ubuf.opacity;
}

