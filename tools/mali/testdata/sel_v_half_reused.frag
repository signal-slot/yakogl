/* Varyings (L3 reference): vc + vec4(vt, vt): the converted vec2 read by both pairs (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float;
varying vec4 vc;
varying vec2 vt;
void main() {
    gl_FragColor = vc + vec4(vt, vt);
}
