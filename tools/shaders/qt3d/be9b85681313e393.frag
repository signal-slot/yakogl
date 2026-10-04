#version 300 es
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

layout(location = 0) out vec4 fragColor;
in vec2 fTex;
in vec4 fColor;

void main()
{
    fragColor = (texture(_qt_texture, fTex) * fColor) * ubuf.opacity;
}

