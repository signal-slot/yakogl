#version 100
/* TEX_COORD, jellyfish 8ee5's caustics coordinate: lanes of a vec4 varying and a time uniform, two sums of quotients */
precision highp float;
uniform sampler2D tex;
uniform float t;
varying vec4 v;
void main() {
    gl_FragColor = texture2D(tex, vec2(v.x / 24.0 + t / 20.0, (v.z - v.y) / 48.0 + t / 40.0));
}
