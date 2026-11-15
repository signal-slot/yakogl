/* Both columns of the matrix, so each of its four lanes is read. */
precision mediump float;
varying mat2 m;
void main() { gl_FragColor = vec4(m[0], m[1]); }
