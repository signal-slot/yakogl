attribute highp vec4 a_position;
attribute highp vec4 a_x;
varying mediump vec4 x;
void main() { gl_Position = a_position; x = a_x; }
