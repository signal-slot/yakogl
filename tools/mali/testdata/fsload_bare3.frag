/* gl_FragColor.zw = a.zw: the lanes read alone. */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(1.0); gl_FragColor.zw = a.zw; }
