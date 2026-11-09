#version 100
/* #33 plan #336 vs-lookups: the fragment shader of vslookup_149_rank.vert,
 * case 149's (tools/mali/testdata/case149_vsunits.frag's body)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; see
 * docs/notes/mali-vs-lookups.md). */
precision mediump float;
varying vec4 v_c;
void main() {
    gl_FragColor = v_c;
}
