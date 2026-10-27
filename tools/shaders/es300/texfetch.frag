#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D s;
uniform ivec2 p;
uniform int lvl;
out vec4 colour;
void main() { colour = 0.5 * (texelFetch(s, p, lvl) + texelFetch(s, p + ivec2(1, 0), lvl)); }
