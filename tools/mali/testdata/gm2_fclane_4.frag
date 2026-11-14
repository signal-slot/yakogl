#version 100
/* FRAGCOLOR_LANE control: lanes 0-2 are the varying's own, lane 3 is the varying's alpha less a constant */
precision highp float;
varying vec4 v;
void main() {
    gl_FragColor = vec4(v.rgb, v.a - 0.3);
}
