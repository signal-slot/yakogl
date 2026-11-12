/* The alpha is a lane alone, beside lanes of two loads. */
precision mediump float;
varying vec2 p;
varying vec2 q;
varying vec2 r;
varying float f;
void main() { gl_FragColor = vec4(r.x, q.y, p.x, f); }
