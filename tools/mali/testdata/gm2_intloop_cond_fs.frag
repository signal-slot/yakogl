#version 100
/* Three steps of a float conditional, no loop: fract, then an if/else on
 * the value that picks one of two fract factors. */
precision mediump float;
void main()
{
    highp vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    gl_FragColor = vec4(d, d, d, 1.0);
}
