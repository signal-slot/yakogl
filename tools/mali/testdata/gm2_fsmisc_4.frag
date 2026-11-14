precision mediump float;
varying vec4 x;
void main(void) { gl_FragColor = vec4(smoothstep(0.25, 0.75, x.x)); }
