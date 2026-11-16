/* The alpha is a lane alone: ATEST reads its conversion's low half. */
precision mediump float;
varying vec3 c;
varying vec3 d;
varying vec3 e;
varying float f;
void main() { gl_FragColor = vec4(e.x, d.y, c.z, f); }
