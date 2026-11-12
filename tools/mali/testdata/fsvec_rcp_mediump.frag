/* Fragment reciprocal at mediump (family C of the Mali glmark2 refusals),
 * the Mesa 26.2.1 drm-shim reference. */
precision mediump float;
varying vec2 v;
void main() { gl_FragColor = vec4(1.0 / v.x); }
