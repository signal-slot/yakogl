/* Conversion order: the colour's own lanes are converted in the
 * colour's order, z then x. */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(a.z, 0.5, a.x, 1.0); }
