/* Load order: d.y lies in slot 1, so slot 1 is loaded first; d.x lies in slot 0 with c. */
precision mediump float;
varying vec3 c;
varying vec3 d;
void main() { gl_FragColor = vec4(d.y, c.x, d.x, 1.0); }
