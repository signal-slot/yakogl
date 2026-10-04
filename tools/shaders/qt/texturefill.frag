#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    mat4 fillMatrix;
    vec2 boundsSize;
    float qt_Opacity;
};

uniform buf ubuf;

uniform highp sampler2D sourceTexture;

varying vec2 textureCoord;

void main()
{
    gl_FragData[0] = texture2D(sourceTexture, textureCoord) * ubuf.qt_Opacity;
}

