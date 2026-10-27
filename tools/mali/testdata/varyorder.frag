/* gl_test case 209 varyorder (#1386): the varyings in the vertex shader's order (Mesa 26.2.1 on drm-shim; highp). */
#version 100
precision highp float;
varying vec4 v_a;
varying vec2 v_uv;
varying vec4 v_b;
void main() {
    gl_FragColor = v_a * vec4(v_uv, 1.0, 1.0) + v_b;
}
