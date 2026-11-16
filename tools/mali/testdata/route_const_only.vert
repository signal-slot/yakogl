attribute highp vec4 a_pos;
varying mediump vec4 v;
void main() {
    gl_Position = a_pos;
    v = vec4(0.25, 0.5, 0.75, 1.0);
}
