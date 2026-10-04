#version 100

struct buf
{
    mat4 matrix;
    mat4 rotation;
    vec4 color;
    float pattern;
    int projection;
};

uniform buf ubuf;

attribute vec4 v;
varying vec2 pos;
attribute float _qt_order;

void main()
{
    vec4 p = ubuf.matrix * v;
    if (ubuf.projection != 0)
    {
        vec4 proj = ubuf.rotation * p;
        gl_Position = vec4(proj.x, proj.y, 0.0, proj.z);
    }
    else
    {
        gl_Position = p;
    }
    pos = v.xy * 1.37000000476837158203125;
    gl_PointSize = 1.0;
    gl_Position.z = _qt_order * gl_Position.w;
}

