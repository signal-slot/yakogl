#version 100
precision highp float;
#define UNIFORMARRAY_SIZE 14
uniform vec4 frag[UNIFORMARRAY_SIZE];
#define innerCol frag[6]
varying vec2 ftcoord;
varying vec2 fpos;
void main(void) {
    gl_FragColor = innerCol;
}
