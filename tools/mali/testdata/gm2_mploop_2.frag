#version 100
/* lower-mploop: glmark2 function fs 849c's shape (an original reduction):
 * the product of gl_FragCoord's x and y, then five calls of a function
 * whose `sqrt` of a mediump value Mesa computes at fp16. The position is
 * highp where the implementation has highp fragment floats, as glmark2
 * writes it. */
#if defined(GL_FRAGMENT_PRECISION_HIGH)
#define FC_PREC highp
#else
#define FC_PREC
#endif
precision mediump float;
float step3(float d) {
    float t = fract(3.0 * d);
    return fract(sqrt(t * t));
}
void main() {
    FC_PREC vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    d = step3(d);
    d = step3(d);
    d = step3(d);
    d = step3(d);
    d = step3(d);
    gl_FragColor = vec4(d, d, d, 1.0);
}
