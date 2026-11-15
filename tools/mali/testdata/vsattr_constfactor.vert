attribute highp vec4 p; attribute highp vec4 d; attribute highp vec4 b;
varying mediump vec4 v;
void main(){ gl_Position = p; v = b + (-(vec4(d.x * 0.0, 1.0, 2.0, 3.0) * vec4(0.0, -0.0, 0.0, 0.0))) * (-b); }
