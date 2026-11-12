/* Load width: two lanes of a vec4, no aligned pair: .v4, each lane
 * converted alone. */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(a.x, 0.5, a.z, 1.0); }
