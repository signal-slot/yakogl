/* A discard before the colour: the compare's p.y is converted at the compare, the colour's c.x after the DISCARD. */
precision mediump float;
varying vec3 c;
varying vec2 p;
void main() { if (p.y < 0.5) discard; gl_FragColor = vec4(c.x, 0.5, 0.25, 1.0); }
