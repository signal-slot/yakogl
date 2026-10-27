#version 300 es
precision highp float;
precision highp int;
uniform uvec2 u;
uniform vec4 a;
out vec4 colour;
void main()
{
    vec4 c;
    if (u.x < u.y)
        c = a * float(u.x % 7u);
    else
        c = a.wzyx + float(u.y / 3u);
    colour = c;
}
