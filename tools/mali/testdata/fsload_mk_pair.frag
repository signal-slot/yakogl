/* Load width: both lanes of an aligned pair beside constants: .v4,
 * then one V2F32_TO_V2F16 of the pair (refused here). */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(a.z, 0.5, a.w, 0.75); }
