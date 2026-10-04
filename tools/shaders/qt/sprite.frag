#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 matrix;
    vec4 animPos;
    vec3 animData;
    float opacity;
};

uniform buf _34;

uniform highp sampler2D tex;

varying vec4 fTexS;
varying float progress;

void main()
{
    gl_FragData[0] = mix(texture2D(tex, fTexS.xy), texture2D(tex, fTexS.zw), vec4(progress)) * _34.opacity;
}

