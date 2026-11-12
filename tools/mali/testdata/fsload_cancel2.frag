/* A whole identity a.xy * vec2(1.0) folded at the link, then f2f32(f2fmp(a)) is a: the lanes read. */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(0.5); gl_FragColor.xy = a.xy * vec2(1.0); }
