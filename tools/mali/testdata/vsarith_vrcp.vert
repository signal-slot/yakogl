#version 100
attribute vec4 a_pos;
attribute vec4 a_w;
varying vec4 v_t;
void main()
{
    v_t = vec4(1.0 / a_w.x, inversesqrt(a_w.y), 1.0 / a_w.z, inversesqrt(a_w.w));
    gl_Position = a_pos;
}
