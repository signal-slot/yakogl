attribute highp vec4 a_position;
attribute highp vec4 a_a;
varying highp vec2 v;
void main() { gl_Position = a_position; v = a_a.xy; }
