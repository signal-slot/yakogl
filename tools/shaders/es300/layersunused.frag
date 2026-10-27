#version 300 es
precision highp float;
precision highp sampler2DArray;
uniform vec4 tint;
uniform sampler2DArray shadows;
uniform float bias;
in vec2 v_xy;
out vec4 colour;
void main()
{
    colour = tint * step(bias, v_xy.x);
}
