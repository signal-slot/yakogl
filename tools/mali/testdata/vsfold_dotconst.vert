/* The smallest vertex shader that makes an Ffma of three constants: the
 * dot product of two constant vectors (fmul, then ffma). The Mesa 26.2.1
 * drm-shim reference. */
attribute vec4 a;
varying vec4 v;
void main() { v = vec4(dot(vec2(3.0, 4.0), vec2(5.0, 6.0))); gl_Position = a; }
