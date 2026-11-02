#version 100
precision mediump float;
uniform vec4 u_p[64];
void main() {
    vec4 r = u_p[58].y * vec4(1.0, 0.0, 1.0, 1.0);
    r += u_p[56].x * u_p[0];
    r += u_p[56].y * u_p[32];
    r += u_p[56].z * u_p[60];
    r += u_p[56].w * u_p[63];
    gl_FragColor = r;
}
