#version 100
attribute vec4 a_pos;
varying vec4 v_t;
void main() {
    v_t = a_pos;
    gl_Position = a_pos;
}
