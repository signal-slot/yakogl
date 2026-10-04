#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float qt_Opacity;
};

uniform buf ubuf;

uniform highp sampler2D source;
uniform highp sampler2D maskSource;

varying vec2 qt_TexCoord0;

void main()
{
    gl_FragData[0] = (texture2D(source, qt_TexCoord0) * texture2D(maskSource, qt_TexCoord0).w) * ubuf.qt_Opacity;
}

