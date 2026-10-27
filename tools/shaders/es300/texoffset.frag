#version 300 es
precision highp float;
uniform highp sampler2D s;
uniform vec2 uv;
uniform float level;
out vec4 colour;
void main() {
    colour = 0.25 * (textureOffset(s, uv, ivec2(-1, 0)) + textureLodOffset(s, uv, level, ivec2(0, -1))
        + textureLodOffset(s, uv, level, ivec2(1, -1)) + textureLodOffset(s, uv, 0.0, ivec2(7, -8)));
}
