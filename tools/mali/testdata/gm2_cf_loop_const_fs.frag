#version 100
/* glmark2 22c527cb loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false,
 * fragment shader c7eae5f6: a for loop with an int counter from 0 to the
 * constant 5, its body a fract of a constant times the value. */
precision mediump float;
void main() {
    highp vec2 p = gl_FragCoord.xy;
    float t = fract(p.x * p.y * 0.0001);
    for (int k = 0; k < 5; k++)
        t = fract(3.0 * t);
    gl_FragColor = vec4(t, t, t, 1.0);
}
