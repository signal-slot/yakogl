/* Load width: one lane of a vec4 in a highp shader: Mesa loads the
 * lane alone. */
precision highp float;
varying vec4 a;
void main() { gl_FragColor = vec4(a.y, 0.5, 0.25, 1.0); }
