attribute highp vec4 p; attribute highp vec4 d; attribute highp vec4 b;
varying mediump vec4 v;
void main(){ gl_Position = p; v = b + d * vec4(-0.0); }
