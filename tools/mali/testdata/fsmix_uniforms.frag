/* Colour lanes from a mix of two uniforms by a varying product (family G
 * of the Mali glmark2 refusals): the Mesa 26.2.1 drm-shim reference. */
precision mediump float;
uniform vec3 c1;
uniform vec3 c2;
varying vec2 uv;
void main() { gl_FragColor = vec4(mix(c1, c2, uv.x * uv.y), 1.0); }
