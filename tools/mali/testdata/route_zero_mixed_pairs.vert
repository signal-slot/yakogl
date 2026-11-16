attribute vec4 a_pos;
attribute vec4 a_v;
attribute vec4 a_w;
varying vec4 v;
varying vec4 w;
void main() {
    gl_Position = a_pos;
    v = a_v;
    w = a_w;
}
