void main (void)
{
	gl_FragColor = vec4(gl_DepthRange.near, gl_DepthRange.far, gl_DepthRange.diff*0.5 + 0.5, 1.0);
}
