#version 100
precision highp float;
precision highp int;

uniform highp sampler2D qt_Texture;

varying vec2 texCoord;
varying float vertexOpacity;

void main()
{
    gl_FragData[0] = texture2D(qt_Texture, texCoord) * vertexOpacity;
}

