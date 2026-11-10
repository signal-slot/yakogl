precision highp float;
uniform int u_i;
void main() {
    int i = u_i;
    int j = i + 1;
    float arr[3];
    arr[0] = 0.125;
    arr[1] = 0.25;
    arr[2] = 0.5;
    arr[j] = arr[i] * 0.5;
    float b = arr[i] + arr[2];
    gl_FragColor = vec4(0.0, 0.0, b, 1.0);
}
