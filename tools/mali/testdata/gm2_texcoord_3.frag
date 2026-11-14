#version 100
/* TEX_COORD: the coordinate is the zw lanes of a vec4 varying */
precision highp float;
uniform sampler2D tex;
varying vec4 v;
void main() {
    gl_FragColor = texture2D(tex, v.zw);
}
