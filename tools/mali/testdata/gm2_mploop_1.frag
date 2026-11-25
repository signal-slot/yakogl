#version 100
/* lower-mploop: glmark2 loop fs 3678's shape (an original reduction): the
 * product of gl_FragCoord's x and y, then a mediump loop to a uniform int
 * bound that Mesa keeps, its fp16 value carried across the back edge. The
 * position is highp where the implementation has highp fragment floats, as
 * glmark2 writes it. */
#if defined(GL_FRAGMENT_PRECISION_HIGH)
#define FC_PREC highp
#else
#define FC_PREC
#endif
precision mediump float;
uniform int n;
void main() {
    FC_PREC vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    for (int i = 0; i < n; i++)
        d = fract(3.0 * d);
    gl_FragColor = vec4(d, d, d, 1.0);
}
