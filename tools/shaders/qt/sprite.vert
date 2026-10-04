#version 100

struct buf
{
    mat4 matrix;
    vec4 animPos;
    vec3 animData;
    float opacity;
};

uniform buf _14;

varying float progress;
varying vec4 fTexS;
attribute vec2 vTex;
attribute vec2 vPos;

void main()
{
    progress = _14.animData.z;
    vec2 _38 = _14.animPos.xy + (vTex * _14.animData.xy);
    fTexS.x = _38.x;
    fTexS.y = _38.y;
    vec2 _53 = _14.animPos.zw + (vTex * _14.animData.xy);
    fTexS.z = _53.x;
    fTexS.w = _53.y;
    gl_Position = _14.matrix * vec4(vPos.x, vPos.y, 0.0, 1.0);
}

