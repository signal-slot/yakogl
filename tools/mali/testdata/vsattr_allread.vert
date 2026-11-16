attribute highp vec4 p; attribute highp vec4 a; attribute highp vec4 b;
varying mediump vec4 v;
void main(){ gl_Position = p; v = a * b; }
