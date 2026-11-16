/* Mesa's link merges the equal lanes z and x and loads .v2; the link
 * here refuses the vertex shader (VaryingDuplicate). */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(a.z, 0.5, 0.25, 1.0); }
