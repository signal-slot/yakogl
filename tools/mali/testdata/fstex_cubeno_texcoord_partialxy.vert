attribute highp vec4 a_position;
attribute highp vec4 a_a3;
varying mediump vec3 a3;
void main() {
    gl_Position = a_position;
    a3 = a_a3.xyz;
}
