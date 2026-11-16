#version 300 es
precision highp float; uniform highp samplerCube s; in float b1; out vec4 o;
void main() {
    o = textureLod(s, vec3(1.0, b1, 0.5), 0.0);
}
