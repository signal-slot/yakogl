attribute highp vec4 a_position;
attribute highp vec4 a_dist;
varying mediump vec4 dist;
void main() { gl_Position = a_position; dist = a_dist; }
