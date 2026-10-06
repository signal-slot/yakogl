#version 100
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    gl_PointSize = 4.0;
    v_color = a_color;
}
