attribute highp vec4 p;
varying highp vec4 v;
attribute mediump float a; attribute mediump float b; attribute mediump float c; attribute mediump float d;
void main() { gl_Position = p; v = vec4(a + b + c + d); }
