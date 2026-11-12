attribute highp vec4 a_position;
attribute highp vec4 a_a2;
varying highp vec4 v;
void main() { gl_Position = a_position; v = a_a2; }
