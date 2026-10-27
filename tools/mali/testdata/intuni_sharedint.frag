#version 100
precision highp float;
precision highp int;

struct Params
{
    mat4 matrix;
    mat4 rotation;
    vec4 color;
    float scale;
    int mode;
};

uniform Params u_p;

varying vec2 v_xy;

void main()
{
    vec4 c = u_p.color;
    if (u_p.mode != 0)
        c.rgb *= 0.5 + 0.5 * sin(v_xy.x + v_xy.y);
    gl_FragColor = c;
}
