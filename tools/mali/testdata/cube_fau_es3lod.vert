#version 300 es
in highp vec4 a_position;
in highp vec4 a_a2;
out mediump vec2 a2;
void main() {
    gl_Position = a_position;
    a2 = a_a2.xy;
}
