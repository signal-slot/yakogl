#version 100
/* glmark2 22c527cb loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true,
 * fragment shader 3678074e: a for loop with an int counter from 0 whose
 * bound is an int uniform, its body a fract of a constant times the value. */
precision mediump float;
uniform int u_n;
void main() {
    highp vec2 p = gl_FragCoord.xy;
    float t = fract(p.x * p.y * 0.0001);
    for (int k = 0; k < u_n; k++)
        t = fract(3.0 * t);
    gl_FragColor = vec4(t, t, t, 1.0);
}
