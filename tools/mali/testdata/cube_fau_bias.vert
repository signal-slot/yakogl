attribute highp vec4 a_position;
attribute highp vec4 a_b1;
varying mediump float b1;
void main() {
    gl_Position = a_position;
    b1 = a_b1.x;
}
