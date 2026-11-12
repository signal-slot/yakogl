/* Fragment min with a constant at mediump (family C of the Mali glmark2
 * refusals), the Mesa 26.2.1 drm-shim reference. */
precision mediump float;
varying vec4 v;
void main() { gl_FragColor = min(v, vec4(0.5)); }
