#version 100

struct buf
{
    mat4 projection;
    mat4 view;
};

uniform buf ubuf;

varying vec3 localPos;
attribute vec3 attr_pos;

void main()
{
    localPos = attr_pos;
    gl_Position = (ubuf.projection * ubuf.view) * vec4(localPos, 1.0);
}

