#version 100

struct Params
{
    mat4 matrix;
    mat4 rotation;
    vec4 color;
    float scale;
    int mode;
};

uniform Params u_p;

attribute vec4 a_pos;

varying vec2 v_xy;

void main()
{
    vec4 p = u_p.matrix * a_pos;
    if (u_p.mode != 0)
    {
        vec4 r = u_p.rotation * p;
        gl_Position = vec4(r.xy, 0.0, r.z);
    }
    else
    {
        gl_Position = p;
    }
    v_xy = a_pos.xy * u_p.scale;
}
