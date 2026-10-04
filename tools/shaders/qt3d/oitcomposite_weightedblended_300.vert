#version 300 es

layout(location = 0) in vec3 attr_pos;
out vec2 uv_coord;
layout(location = 1) in vec2 attr_uv;

void main()
{
    vec3 p = attr_pos;
    gl_Position = vec4(p, 1.0);
    uv_coord = attr_uv;
}

