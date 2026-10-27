#version 300 es
precision highp float;
precision highp int;
uniform ivec2 v;
out vec4 colour;
void main() {
    int s = v.x >> v.y;
    colour = vec4(float(s & 255) / 255.0, float((s >> 8) & 255) / 255.0, s < 0 ? 1.0 : 0.0, 1.0);
}
