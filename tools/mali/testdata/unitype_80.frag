precision highp float;
uniform vec4 u_k;
uniform float u_c;
uniform int u_i;
void main() {
    mat2 m = mat2(u_k.x, u_k.y, u_k.z, u_k.w);
    mat2 s = m + m;
    mat2 d = 4.0 - m;
    mat2 q = 2.0 / m;
    mat2 n = -m;
    mat2 p = m++;
    m *= m;
    float r = (s[1][1] + d[1][0] + q[0][1] + n[1][0] + 4.0 * p[1][0]) / 16.0;

    vec2 t = (u_c > 0.5) ? vec2(0.5, 0.0) : vec2(0.0, 0.25);
    float g = (m[1][0] - m[0][0]) / 8.0 + t.x + t.y;

    int i = u_i;
    int j = i + 1;
    float arr[3];
    arr[0] = 0.125;
    arr[1] = 0.25;
    arr[2] = 0.5;
    arr[j] = arr[i] * 0.5;
    vec2 col = p[i];
    vec4 v = vec4(0.0, 0.125, 0.25, 0.375);
    float b = arr[i] + col[i - 1] * 0.25 + v[j] + arr[2];

    gl_FragColor = vec4(r, g, b, 1.0);
}
