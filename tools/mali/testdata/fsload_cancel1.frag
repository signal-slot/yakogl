/* A mediump local of the varying read back bare: the link's f2f32(f2fmp(a)) is a, so the lanes read (.v2). */
precision mediump float;
varying vec4 a;
void main() { mediump vec4 t = a; gl_FragColor = vec4(1.0); gl_FragColor.xy = t.xy; }
