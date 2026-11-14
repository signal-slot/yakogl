#version 100
/* gm2_cf_ifuni_vs: the fragment stage draws the varying. */
precision mediump float;
varying vec4 v_col;
void main() {
    gl_FragColor = v_col;
}
