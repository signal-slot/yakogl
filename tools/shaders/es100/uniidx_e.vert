// gl_test 116 uniidxe: gl_test 100 idxtex's vertex-stage shape in ES 1.00 --
// uniforms and a run-time-indexed uniform read of its own -- under 108 D's
// fragment shader (uniidx_d.frag), which fails on the i.MX8MP with a
// passthrough vertex shader while idxtex passes.
attribute vec4 a_pos;
uniform vec4 vcols[4];
uniform int vidx;
varying vec4 v_color;
void main()
{
    gl_Position = a_pos;
    v_color = vcols[vidx];
}
