/* An identity on the whole operation: p.x + 0.0 is folded at the link; q.y, p.x, c.z converted at the store in that order. */
precision mediump float;
varying vec3 c;
varying vec2 p;
varying vec2 q;
void main() { gl_FragColor = vec4(q.y, p.x + 0.0, c.z, 1.0); }
