#version 100

struct buf
{
    mat4 viewProjection;
};

uniform buf ubuf;

varying vec3 var_color;
attribute vec3 attr_color;
attribute vec3 attr_pos;

void main()
{
    var_color = attr_color;
    gl_Position = ubuf.viewProjection * vec4(attr_pos, 1.0);
    gl_PointSize = 4.0;
}

