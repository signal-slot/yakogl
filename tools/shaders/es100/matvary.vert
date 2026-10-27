/* A matrix varying, and a varying declared after it. A mat2 varying is one
 * declaration over two locations, one per column; the online compiler
 * counted declarations and refused it ("a varying written that the module
 * does not declare", about 250 dEQP case-sides). It now takes a register per
 * column, and v_after moves up by one. gl_test case 78. */
attribute vec4 a_pos;
attribute vec4 a_color;
varying mat2 v_m;
varying vec4 v_after;
void main()
{
	v_m = mat2(a_color.x, a_color.y, a_color.z, a_color.w);
	v_after = vec4(0.5);
	gl_Position = a_pos;
}
