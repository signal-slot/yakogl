/* Load width: dFdy of one lane of a vec2: .v2, then that lane
 * converted alone (refused here). */
#extension GL_OES_standard_derivatives : enable
precision mediump float;
varying vec2 v;
void main() { gl_FragColor = vec4(dFdy(v.y), 0.5, 0.5, 1.0); }
