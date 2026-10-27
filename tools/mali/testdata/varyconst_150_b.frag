#version 100
// gl_test 150 shprobe, both arms: gl_test_vsunits_fs_src (gl_test_shaders.h).
precision mediump float;
varying vec4 v_c;
void main() {
    gl_FragColor = v_c;
}
