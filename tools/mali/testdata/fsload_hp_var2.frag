/* A colour that holds a highp varying's lane is not lowered: a.x and
 * b.y are kept alone (.v2, two stores). */
precision mediump float;
varying vec4 a;
varying highp vec4 b;
void main() { gl_FragColor = vec4(a.x, b.y, 0.0, 1.0); }
