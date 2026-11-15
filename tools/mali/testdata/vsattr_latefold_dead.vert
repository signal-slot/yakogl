attribute highp vec4 p; attribute highp vec4 d; attribute highp vec4 b;
varying mediump vec4 v;
void main(){ vec4 t = d * vec4(0.0, -0.0, 0.0, 0.0); gl_Position = p; v = b; }
