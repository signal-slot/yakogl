#version 100
attribute vec4 a_pos;
attribute vec3 a_normal;
varying vec3 v_normal;
varying vec4 v_pos;
void main()
{
    v_normal = a_normal;
    v_pos = a_pos;
    gl_Position = a_pos;
}
