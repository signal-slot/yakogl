#version 100

varying vec2 uv_coord;
attribute vec2 attr_uv;
attribute vec3 attr_pos;

void main()
{
    uv_coord = attr_uv;
    gl_Position = vec4(attr_pos, 1.0);
}

