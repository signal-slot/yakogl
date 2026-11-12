/* An assignment to the colour itself, gl_FragColor.xy = a.xy: no tree to lower, so the link keeps the lanes read (.v2, two stores). */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(1.0); gl_FragColor.xy = a.xy; }
