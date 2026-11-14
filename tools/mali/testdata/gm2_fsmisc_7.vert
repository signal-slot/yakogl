attribute highp vec4 a_position;
attribute highp vec4 a_a;
attribute highp vec4 a_b;
attribute highp float a_t;
varying mediump vec4 a;
varying mediump vec4 b;
varying mediump float t;
void main() { gl_Position = a_position; a = a_a; b = a_b; t = a_t; }
