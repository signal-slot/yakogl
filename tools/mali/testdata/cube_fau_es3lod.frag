#version 300 es
precision mediump float; uniform samplerCube s; in vec2 a2; out vec4 o;
void main() {
    o = textureLod(s, vec3(a2, 0.5), 0.0);
}
