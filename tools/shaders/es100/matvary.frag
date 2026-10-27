/* Both columns of the matrix varying and the varying after it: with
 * a_color = (0.25, 0.0, 0.0, 0.75) the colour is
 * (v_m[0].x, v_m[1].y, v_after.z, 1.0) = (0.25, 0.75, 0.5, 1.0). gl_test case 78. */
precision mediump float;
varying mat2 v_m;
varying vec4 v_after;
void main()
{
	gl_FragColor = vec4(v_m[0].x, v_m[1].y, v_after.z, 1.0);
}
