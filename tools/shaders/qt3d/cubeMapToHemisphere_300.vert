#version 300 es

out vec2 uv_coord;
layout(location = 1) in vec2 attr_uv;
layout(location = 0) in vec3 attr_pos;

void main()
{
    uv_coord = attr_uv;
    gl_Position = vec4(attr_pos, 1.0);
}

