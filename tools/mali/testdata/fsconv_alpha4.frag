/* The alpha d.x is a lane alone of the second load. */
precision mediump float;
varying float f;
varying vec3 c;
varying vec2 p;
varying vec3 d;
void main() { gl_FragColor = vec4(f, c.z, p.x, d.x); }
