/* A discard of p.x, then the colour's c.y after the DISCARD; its p.x is the compare's conversion. */
precision mediump float;
varying vec3 c;
varying vec2 p;
void main() { if (p.x > 0.5) discard; gl_FragColor = vec4(p.x, c.y, 0.5, 1.0); }
