/* Packing order: the link lays the vec2 before the vec3 (q.xy,p.xy in
 * slot 0, p.z in slot 1), so p.z is loaded alone from slot 1. */
precision mediump float;
varying vec3 p;
varying vec2 q;
void main() { gl_FragColor = vec4(p.z, 0.5, q.x, 1.0); }
