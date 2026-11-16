/* Load width: a lane of a vec2 and a lane of a vec4: .v2 and .v4. */
precision mediump float;
varying vec4 a;
varying vec2 v;
void main() { gl_FragColor = vec4(v.x, 0.5, a.x, 0.75); }
