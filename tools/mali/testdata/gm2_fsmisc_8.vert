attribute highp vec4 a_position;
attribute highp vec4 a_x;
attribute highp vec4 a_y;
varying mediump vec4 x;
varying mediump vec4 y;
void main() { gl_Position = a_position; x = a_x; y = a_y; }
