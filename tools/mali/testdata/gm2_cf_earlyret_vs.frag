#version 100
/* gm2_cf_earlyret_vs: the fragment stage reads the three varyings. */
precision mediump float;
varying vec3 v_nrm;
varying vec4 v_eye;
varying vec3 v_dir;
void main() {
    gl_FragColor = vec4(v_nrm + v_dir, v_eye.w);
}
