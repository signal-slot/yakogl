attribute highp vec4 a_position;
attribute highp vec4 a_b;
uniform samplerCube s;
varying mediump vec4 c;
void main() {
    gl_Position = a_position;
    c = textureCubeLod(s, vec3(1.0, a_b.x, 0.5), 0.0);
}
