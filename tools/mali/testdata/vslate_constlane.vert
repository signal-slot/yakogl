attribute highp vec4 p; attribute highp vec4 d; attribute highp vec4 b;
varying mediump vec4 v; varying mediump vec4 w;
void main(){ gl_Position = p; v = b + d * vec4(0.0, -0.0, 0.0, 0.0); w = vec4(2.0, 3.0, p.x, p.y) * vec4(0.0, -0.0, 1.0, 1.0); }
