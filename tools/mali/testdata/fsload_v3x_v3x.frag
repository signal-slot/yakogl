/* Load per slot: p.x and q.x are both in slot 0, one .v4 load; slot 1
 * (q.yz) is stored by the vertex shader and not loaded. */
precision mediump float;
varying vec3 p;
varying vec3 q;
void main() { gl_FragColor = vec4(p.x, q.x, 0.5, 1.0); }
