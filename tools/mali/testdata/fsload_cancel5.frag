/* A local of the default precision read back bare: the lanes read. */
precision mediump float;
varying vec4 a;
void main() { vec4 t = a; gl_FragColor = vec4(1.0); gl_FragColor.xy = t.xy; }
