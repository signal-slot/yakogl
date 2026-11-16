attribute vec4 a_pos;
attribute float a_e;
varying mediump float e;
void main() {
    gl_Position = a_pos;
    e = a_e;
}
