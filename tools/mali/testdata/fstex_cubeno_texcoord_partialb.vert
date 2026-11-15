attribute highp vec4 a_position;
attribute highp vec4 a_a2;
attribute highp vec4 a_b2;
varying mediump vec2 a2;
varying mediump vec2 b2;
void main() {
    gl_Position = a_position;
    a2 = a_a2.xy;
    b2 = a_b2.xy;
}
