attribute highp vec4 p; attribute highp vec4 a; attribute highp vec4 b;
varying mediump vec4 v;
void main(){ highp vec4 d = a * 2.0; gl_Position = p; v = b; }
