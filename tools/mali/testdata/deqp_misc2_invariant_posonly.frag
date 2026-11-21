/* Not dEQP's text: a constant mediump colour, so the vertex program alone
 * decides the outcome (deqp_misc2_invariant_posonly.vert). */
precision mediump float;
varying mediump vec4 v_unrelated;
void main ()
{
    gl_FragColor = vec4(0.25, 0.5, 0.75, 1.0);
}
