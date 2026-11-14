#version 100
/* glmark2 22c527cb conditionals:vertex-steps=0:fragment-steps=5, fragment
 * shader a5b5e746: five two-armed `if`s in a row on a mediump float compare
 * with a constant, each arm a fract of a constant times the value; the
 * value starts from the highp gl_FragCoord. */
precision mediump float;
void main() {
    highp vec2 p = gl_FragCoord.xy;
    float t = fract(p.x * p.y * 0.0001);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    gl_FragColor = vec4(t, t, t, 1.0);
}
