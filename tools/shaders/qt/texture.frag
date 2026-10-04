#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float opacity;
};

uniform buf _23;

uniform highp sampler2D qt_Texture;

varying vec2 qt_TexCoord;

void main()
{
    gl_FragData[0] = texture2D(qt_Texture, qt_TexCoord) * _23.opacity;
}

