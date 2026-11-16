/* Conversion order: the discard's compare converts y first, then the
 * colour x; Mesa makes them one V2F32_TO_V2F16 with y low (refused). */
precision mediump float;
varying vec4 a;
void main() { if (a.y > 0.5) discard; gl_FragColor = vec4(a.x, 0.5, 0.0, 1.0); }
