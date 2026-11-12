/* Load width: one lane of a highp vec4 in a mediump shader: Mesa
 * loads the lane alone. */
precision mediump float;
varying highp vec4 a;
void main() { gl_FragColor = vec4(a.x, 0.5, 0.25, 1.0); }
