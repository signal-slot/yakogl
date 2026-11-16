/* Load per slot: q.z (slot 1) is the colour's first lane, so slot 1 is
 * loaded first (.v2), then slot 0 (.v4) of which only p.x is read. */
precision mediump float;
varying vec3 p;
varying vec3 q;
void main() { gl_FragColor = vec4(q.z, p.x, 0.5, 1.0); }
