attribute highp vec4 a_position;
attribute highp vec4 a_v3;
varying mediump vec3 v3;
void main() {
    gl_Position = a_position;
    v3 = a_v3.xyz;
}
