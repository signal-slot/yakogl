/* Load width: one lane of a mediump vec3: Mesa loads .v3. */
precision mediump float;
varying vec3 t;
void main() { gl_FragColor = vec4(t.y, 0.5, 0.25, 1.0); }
