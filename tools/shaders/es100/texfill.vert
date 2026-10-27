#version 100

struct Fill
{
    mat4 matrix;
    mat4 fill;
    vec2 size;
    float opacity;
};

uniform Fill u_fill;

attribute vec4 a_pos;
varying vec2 v_uv;
attribute float a_order;
attribute vec4 a_color;

void main()
{
    vec2 p = (u_fill.fill * a_pos).xy;
    v_uv = vec2(p.x / u_fill.size.x, p.y / u_fill.size.y);
    gl_Position = u_fill.matrix * a_pos;
    gl_Position.z = a_order * gl_Position.w;
}
