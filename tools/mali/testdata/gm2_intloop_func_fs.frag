#version 100
/* A step in a function, called twice in each of two forms: the low form
 * (one fract) and the medium form (a product of two fracts, then fract of
 * a square root). */
precision mediump float;
float low_step(float d)
{
    d = fract(3.0 * d);
    return d;
}
float medium_step(float d)
{
    d = fract(3.0 * d);
    d = fract(3.0 * d) * fract(3.0 * d);
    d = fract(sqrt(d));
    return d;
}
void main()
{
    highp vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    d = low_step(d);
    d = low_step(d);
    d = medium_step(d);
    d = medium_step(d);
    gl_FragColor = vec4(d, d, d, 1.0);
}
