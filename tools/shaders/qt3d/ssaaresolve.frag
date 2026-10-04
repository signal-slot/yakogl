#version 100
precision highp float;
precision highp int;

uniform highp sampler2D tex;

varying vec2 uv_coord;

void main()
{
    gl_FragData[0] = texture2D(tex, uv_coord);
}

