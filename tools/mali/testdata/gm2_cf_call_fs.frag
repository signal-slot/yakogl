#version 100
/* glmark2 22c527cb function:fragment-steps=5:fragment-complexity=low,
 * fragment shader c8ce6c64 (and, with a longer body, =medium, 849c7814):
 * five calls in a row of a mediump float -> float function whose body
 * reassigns its parameter and returns it. */
precision mediump float;
float step3(float x) {
    x = fract(3.0 * x);
    return x;
}
void main() {
    highp vec2 p = gl_FragCoord.xy;
    float t = fract(p.x * p.y * 0.0001);
    t = step3(t);
    t = step3(t);
    t = step3(t);
    t = step3(t);
    t = step3(t);
    gl_FragColor = vec4(t, t, t, 1.0);
}
