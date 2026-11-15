attribute highp vec4 p; attribute highp mat2 m; attribute highp vec4 b;
varying mediump vec4 v;
void main(){ gl_Position = p; v = vec4(m[1], b.xy); }
