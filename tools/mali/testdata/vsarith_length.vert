/* Vertex length of a vec3, a square root (family B of the Mali glmark2
 * refusals): the Mesa 26.2.1 drm-shim reference. */
attribute vec4 a;
varying vec4 v;
void main() { v = vec4(length(a.xyz)); gl_Position = a; }
