attribute highp vec4 a_position;
attribute highp vec4 a_v4;
varying mediump vec4 v4;
void main() {
    gl_Position = a_position;
    v4 = a_v4.xyzw;
}
