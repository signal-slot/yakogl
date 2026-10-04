#version 100
/* gl_FrontFacing (dEQP shaders.builtin_variable.frontfacing's use of it):
 * one colour where the primitive faces front, another where it faces back.
 * The harness makes some lanes back-facing (REVF 1). */
precision mediump float;
varying mediump vec4 v_coords;
void main()
{
    if (gl_FrontFacing)
        gl_FragColor = vec4(v_coords.xy, 0.25, 1.0);
    else
        gl_FragColor = vec4(0.75, v_coords.zw, 1.0);
}
