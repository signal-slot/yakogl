#version 100
/* Case 91: a quarter of an RGBA16F texel, so values above 1.0 stay visible in
 * an 8-bit target. */
precision mediump float;
uniform sampler2D hdr;
varying vec2 v_uv;
void main() { gl_FragColor = texture2D(hdr, v_uv) * 0.25; }
