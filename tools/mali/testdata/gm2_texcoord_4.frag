#version 100
/* TEX_COORD: two look-ups, the coordinates the xy and the zw lanes of one vec4 varying, summed */
precision highp float;
uniform sampler2D s0;
uniform sampler2D s1;
varying vec4 v;
void main() {
    gl_FragColor = texture2D(s0, v.xy) + texture2D(s1, v.zw);
}
