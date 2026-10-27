#version 300 es
precision highp float;
precision highp int;
uniform vec4 u;
uniform ivec2 i;
uniform uvec2 w;
in vec4 v_t;
out vec4 colour;
void main()
{
    float d0 = dot(u.xyz, v_t.xyz);
    float d1 = dot(v_t.xyz, vec3(0.299, 0.587, 0.114)) + dot(v_t.yzw, vec3(0.5));
    float f = floor(u.x) + sqrt(u.y);
    int a = abs(i.x);
    uint x = w.x ^ uint(a);
    colour = vec4(d0, d1, f, float(x & 255u) / 255.0);
}
