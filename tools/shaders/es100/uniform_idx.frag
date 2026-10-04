#version 100
/* A uniform array at an index read at run time (v3d back end:
 * LoadUniformIdx as a select over its elements). The index differs from
 * fragment to fragment and runs past both ends (-2 .. 10 over 5 elements),
 * so the clamp (ir.md, eval.rs) is exercised on both sides; a second read
 * takes its index from a uniform. */
precision mediump float;
uniform vec4 u_arr[5];
uniform float u_k;
varying vec4 v_color;
void main() {
    int i = int(v_color.r * 8.0) - 2;
    int j = int(u_k * 4.0);
    gl_FragColor = u_arr[i] * 0.5 + u_arr[j].wzyx * 0.25 + vec4(0.0, 0.0, 0.0, 0.25);
}
