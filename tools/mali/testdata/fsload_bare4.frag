/* A constructor's p.x (lowered: p kept whole), then gl_FragColor.y = c.y (c.y alone). */
precision mediump float;
varying vec3 c; varying vec2 p;
void main() { gl_FragColor = vec4(p.x, 0.5, 0.5, 1.0); gl_FragColor.y = c.y; }
