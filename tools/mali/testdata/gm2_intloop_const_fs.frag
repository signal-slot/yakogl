#version 100
/* A fragment loop of a constant trip count (a counted `for` of int): five
 * steps of fract(3.0 * d) on a value made of gl_FragCoord, then one step
 * after the loop. The shape of a benchmark's fragment loop with a
 * compile-time bound. */
precision mediump float;
void main()
{
    highp vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    for (int i = 0; i < 5; i++)
        d = fract(3.0 * d);
    d = fract(3.0 * d);
    gl_FragColor = vec4(d, d, d, 1.0);
}
