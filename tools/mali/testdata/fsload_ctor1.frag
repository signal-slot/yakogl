/* A constructor beside constants, read back in the varying's lanes only (t.xy): the lanes read (.v2). */
precision mediump float;
varying vec4 a;
void main() { vec4 t = vec4(a.xy, 0.5, 1.0); gl_FragColor = vec4(1.0); gl_FragColor.xy = t.xy; }
