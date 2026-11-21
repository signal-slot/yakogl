/* Not dEQP's text: the gl_Position of
 * dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_0
 * (deqp_misc2_invariant_cse.vert) with a constant v_unrelated, the shape of
 * the case's second program. Shape invariant_arith of
 * docs/notes/mali-deqp-misc2.md. */
attribute highp vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    v_unrelated = vec4(0.0);
    gl_Position = a_input + (1.0e20*a_input.x*a_input.xxxx + 1.0e20*a_input.y*a_input.yyyy) * 1.0e-20;
}
