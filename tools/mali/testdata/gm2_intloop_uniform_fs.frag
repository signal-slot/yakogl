#version 100
/* The same loop with its trip count a uniform int, as the benchmark writes
 * it (ES 1.00 asks for a constant bound; Mesa accepts the uniform): nothing
 * unrolls. With a large count this is the board's hang-risk shape. */
precision mediump float;
uniform int u_loops;
void main()
{
    highp vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    for (int i = 0; i < u_loops; i++)
        d = fract(3.0 * d);
    d = fract(3.0 * d);
    gl_FragColor = vec4(d, d, d, 1.0);
}
