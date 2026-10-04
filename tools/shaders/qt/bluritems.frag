#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float qt_Opacity;
    vec2 offset;
};

uniform buf _44;

uniform highp sampler2D source;

varying vec2 texCoord0;
varying vec2 texCoord1;
varying vec2 texCoord2;
varying vec2 texCoord3;

void main()
{
    vec4 sourceColor = (((texture2D(source, texCoord0) + texture2D(source, texCoord1)) + texture2D(source, texCoord2)) + texture2D(source, texCoord3)) / vec4(4.0);
    gl_FragData[0] = sourceColor * _44.qt_Opacity;
}

