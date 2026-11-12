/* Load per slot: q.z (slot 1) first, then p.z (slot 0). */
precision mediump float;
varying vec3 p;
varying vec3 q;
void main() { gl_FragColor = vec4(q.z, p.z, 0.5, 1.0); }
