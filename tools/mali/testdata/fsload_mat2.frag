/* A matrix is converted per column: m[0] is kept whole (.v2, two
 * stores), m[1] is not read and not kept. */
precision mediump float;
varying mat2 m;
void main() { gl_FragColor = vec4(m[0].y, 0.5, 0.25, 1.0); }
