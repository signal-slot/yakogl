#version 100
/* dEQP shaders.return.return_in_dynamic_loop_never_fragment
 * (es2fShaderReturnTests.cpp makeReturnInLoopCase, Apache-2.0): a loop
 * with a uniform bound (ui_one = 1, as dEQP sets it) whose body swaps two
 * pairs of lanes, x <-> w and y <-> z, and a return that never happens.
 * dEQP writes vec4(getCoords().xyz, 1.0); this writes all four lanes, so
 * none the loop carries is dead after it (a mutant of the x varying's
 * interpolation otherwise changed nothing the harness compares). */
precision mediump float;
varying mediump vec4 v_coords;
uniform mediump int ui_one;

mediump vec4 getCoords (void)
{
    mediump vec4 coords = v_coords;
    for (int i = 0; i < ui_one; i++)
    {
        if (false)
            return coords;
        coords = coords.wzyx;
    }
    return coords;
}

void main (void)
{
    gl_FragColor = getCoords();
}
