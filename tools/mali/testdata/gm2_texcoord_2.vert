#version 100
/* ref-texcoord-lane: pass-through position and one vec4 varying v */
attribute vec4 a_pos;
attribute vec4 a_v;
varying vec4 v;
void main() {
    gl_Position = a_pos;
    v = a_v;
}
