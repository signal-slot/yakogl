/* gl_FragColor.x = c.y: the lane read alone. */
precision mediump float;
varying vec3 c; varying vec2 p;
void main() { gl_FragColor.x = c.y; gl_FragColor.yzw = vec3(0.5); }
