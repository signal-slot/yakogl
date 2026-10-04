#version 300 es
precision highp float;
precision highp int;

struct buf
{
    vec4 clearColor0;
    vec4 clearColor1;
};

uniform buf ubuf;

layout(location = 0) out vec4 fragOutput0;
layout(location = 1) out vec4 fragOutput1;

void main()
{
    fragOutput0 = ubuf.clearColor0;
    fragOutput1 = ubuf.clearColor1;
}

