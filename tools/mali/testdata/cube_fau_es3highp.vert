#version 300 es
in highp vec4 a_position;
in highp vec4 a_b1;
out highp float b1;
void main() {
    gl_Position = a_position;
    b1 = a_b1.x;
}
