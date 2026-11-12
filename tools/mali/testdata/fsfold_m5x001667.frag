/* Mediump product of two constants whose fp32 result fp16 does not hold
 * exactly (-5.0 x 0.001667, family D of the Mali glmark2 refusals): the
 * Mesa 26.2.1 drm-shim reference. */
precision mediump float;
varying vec2 uv;
void main() { const float k = 0.001667; gl_FragColor = vec4(uv.x + -5.0 * k); }
