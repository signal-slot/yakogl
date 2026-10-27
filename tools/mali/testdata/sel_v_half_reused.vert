/* Varyings (L3 reference): the VS of sel_v_half_reused (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 p;
attribute vec4 c;
attribute vec2 t;
varying vec4 vc;
varying vec2 vt;
void main() {
    gl_Position = p;
    vc = c;
    vt = t;
}
