/* A vertex normalize of a constant vector, a light direction's shape: its
 * dot product is an Ffma of three constants. The Mesa 26.2.1 drm-shim
 * reference. */
attribute vec4 a;
varying vec4 v;
void main() { v = vec4(normalize(vec3(20.0, 20.0, 10.0)), 1.0); gl_Position = a; }
