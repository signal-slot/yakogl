#pragma STDGL invariant(all)
precision highp float;
uniform vec4 u_q, u_b;
void main() { gl_FragColor = (u_b * 1.0) * u_q * 8.0; }
