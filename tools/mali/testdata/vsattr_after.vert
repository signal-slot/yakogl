attribute highp vec4 p; attribute highp vec4 b; attribute highp vec4 z;
varying mediump vec4 v;
void main(){ gl_Position = p; v = b; }
