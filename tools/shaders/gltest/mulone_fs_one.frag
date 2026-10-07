#version 100
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() { gl_FragColor = vec4(mat3(vec3(u_q, u_a, 0.0), vec3(u_a, u_a, u_q), vec3(u_b, -u_q, u_x)) * vec3(0.5, 0.5, 1.0), 1.0); }
