attribute vec4 a_pos;
attribute vec4 a_w;
attribute vec4 a_v;
varying vec4 w;
varying vec2 v;
void main() {
    gl_Position = a_pos;
    w = a_w;
    v = a_v.xy;
}
