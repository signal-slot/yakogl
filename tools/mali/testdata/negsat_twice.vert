attribute highp vec4 a_position;
attribute highp vec4 a_a2;
varying mediump vec2 a2;
varying mediump vec2 v;
void main() { gl_Position = a_position; a2 = a_a2.xy; v = a_a2.zw; }
