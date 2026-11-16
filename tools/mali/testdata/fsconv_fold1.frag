/* An identity on the whole operation: a.x * 1.0 is folded at the link, so a.x is the colour's own lane, converted after a.z. */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(a.z, a.x * 1.0, 0.5, 1.0); }
