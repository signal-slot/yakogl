#version 100
precision highp float;
varying vec2 ftcoord;
void main(void) {
    gl_FragColor = vec4(ftcoord.x, ftcoord.y, 0.0, 1.0);
}
