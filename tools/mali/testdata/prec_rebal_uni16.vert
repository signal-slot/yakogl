attribute highp vec4 p;
varying highp vec4 v;
uniform mediump float a; uniform mediump float b; uniform mediump float c; uniform mediump float d;
void main() { gl_Position = p; v = p * (a + b + c + d); }
