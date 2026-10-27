#version 300 es
precision highp float;
precision highp sampler2DArray;
uniform vec4 tint;
uniform sampler2DArray shadows;
uniform float bias;
uniform sampler2D base;
in vec2 v_xy;
out vec4 colour;
void main()
{
    float d = texture(shadows, vec3(v_xy, 1.0)).r;
    colour = texture(base, v_xy) * tint * step(bias, d);
}
