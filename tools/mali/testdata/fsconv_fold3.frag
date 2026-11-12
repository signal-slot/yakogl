/* An identity on every lane of a vector operation: (p * 1.0).x is the colour's own lane. */
precision mediump float;
varying vec3 c;
varying vec2 p;
varying vec2 q;
void main() { gl_FragColor = vec4(q.y, (p * 1.0).x, c.z, 1.0); }
