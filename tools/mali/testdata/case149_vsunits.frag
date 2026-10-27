#version 100
/* #1384/#1385 plan #153 study-vstex: case 149 fragment shader (tools/shaders/gltest/vsunits_fs.frag, verbatim body)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; see
 * docs/notes/mali-vs-texture-lanes.md). */
precision mediump float;
varying vec4 v_c;
void main() {
    gl_FragColor = v_c;
}
