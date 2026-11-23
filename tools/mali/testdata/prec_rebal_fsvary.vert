attribute highp vec4 p;
attribute highp vec4 pa; attribute highp vec4 pb; attribute highp vec4 pc; attribute highp vec4 pd;
varying mediump vec4 a; varying mediump vec4 b; varying mediump vec4 c; varying mediump vec4 d;
void main() { gl_Position = p; a = pa; b = pb; c = pc; d = pd; }
