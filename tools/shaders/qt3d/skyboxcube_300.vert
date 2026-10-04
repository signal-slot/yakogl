#version 300 es

struct buf
{
    vec4 skyboxProperties;
    mat3 orientation;
    mat4 viewProjection;
    mat4 inverseProjection;
    mat3 viewMatrix;
};

uniform buf ubuf;

out vec3 tex_coords;
layout(location = 0) in vec3 attr_pos;

void main()
{
    tex_coords = attr_pos * vec3(-1.0, 1.0, 1.0);
    vec4 pos = ubuf.viewProjection * vec4(attr_pos, 1.0);
    pos.y *= ubuf.skyboxProperties.x;
    gl_Position = pos.xyww;
}

