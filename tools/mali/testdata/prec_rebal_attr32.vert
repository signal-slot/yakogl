attribute highp vec4 p;
varying highp vec4 v;
attribute highp float a; attribute highp float b; attribute highp float c; attribute highp float d;
void main() { gl_Position = p; v = vec4(a + b + c + d); }
