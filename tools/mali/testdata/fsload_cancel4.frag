/* (a * 1.0).xy: a whole identity folded, the lanes read. */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(1.0); gl_FragColor.xy = (a * 1.0).xy; }
