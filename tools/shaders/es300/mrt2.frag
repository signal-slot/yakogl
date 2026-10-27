#version 300 es
precision highp float;

struct Colors
{
    vec4 first;
    vec4 second;
};

uniform Colors u_clear;

layout(location = 0) out vec4 o_first;
layout(location = 1) out vec4 o_second;

void main()
{
    o_first = u_clear.first;
    o_second = u_clear.second;
}
