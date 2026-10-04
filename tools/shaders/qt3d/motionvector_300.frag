#version 300 es
precision highp float;
precision highp int;

struct buf
{
    mat4 mvp;
    mat4 prevMVP;
    mat4 instanceLocal;
    mat4 instanceGlobal;
    mat4 prevInstanceLocal;
    mat4 prevInstanceGlobal;
    vec4 currentAndLastJitter;
    float velocityAmount;
    int morphTargetCount;
    vec2 padding;
};

uniform buf ubuf;

in vec4 vPosition;
in vec4 vPrevPosition;
layout(location = 0) out vec4 fragColor;

void main()
{
    vec2 a = vPosition.xy / vec2(vPosition.w);
    vec2 b = vPrevPosition.xy / vec2(vPrevPosition.w);
    a -= ubuf.currentAndLastJitter.zw;
    b -= ubuf.currentAndLastJitter.xy;
    a = (a * 0.5) - vec2(0.5);
    b = (b * 0.5) - vec2(0.5);
    vec2 v;
    v.x = b.x - a.x;
    v.y = a.y - b.y;
    fragColor = vec4(v * ubuf.velocityAmount, v);
}

