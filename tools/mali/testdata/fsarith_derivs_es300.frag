#version 300 es
precision highp float;
uniform vec4 a;
in vec4 v_t;
out vec4 colour;
void main()
{
    vec4 t = v_t * a;
    colour = vec4(dFdx(t.x), dFdy(t.y), dFdx(t.z) + dFdy(t.w), 1.0);
}
