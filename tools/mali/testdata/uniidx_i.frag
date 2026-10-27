/* #1385 (b): gl_test case 122's fragment shader,
 * tools/shaders/es100/uniidx_i.frag verbatim: a look-up at the constant
 * coordinate vec2(0.5), then a mediump vec4 array read at a run-time index
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1, --uniform-values). */
precision mediump float;
uniform vec4 fcols[4];
uniform int fidx;
uniform sampler2D ftex;
void main() {
    vec4 t = texture2D(ftex, vec2(0.5));
    gl_FragColor = 0.5 * (fcols[fidx + int(t.w * 0.0)] + t);
}
