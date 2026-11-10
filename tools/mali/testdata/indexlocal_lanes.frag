precision highp float;
uniform vec4 u_k;
uniform int u_i;
void main() {
    mat2 p = mat2(u_k.x, u_k.y, u_k.z, u_k.w);
    int i = u_i;
    int j = i + 1;
    vec2 col = p[i];
    vec4 v = vec4(0.0, 0.125, 0.25, 0.375);
    float b = col[i - 1] * 0.25 + v[j];
    gl_FragColor = vec4(0.0, 0.0, b, 1.0);
}
