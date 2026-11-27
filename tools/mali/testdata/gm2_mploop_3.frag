#version 100
/* lower-mploop: function fs 849c's step once (an original reduction): the
 * product of gl_FragCoord's x and y, then `fract(sqrt(..))` of a mediump
 * square. The position is highp where the implementation has highp
 * fragment floats, as glmark2 writes it. */
#if defined(GL_FRAGMENT_PRECISION_HIGH)
#define FC_PREC highp
#else
#define FC_PREC
#endif
precision mediump float;
void main() {
    FC_PREC vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    float t = fract(3.0 * d);
    d = fract(sqrt(t * t));
    gl_FragColor = vec4(d, d, d, 1.0);
}
