/* VS of gm2_locals_noise.frag: the coordinate through one varying. */
attribute vec4 p;
attribute vec3 uv;
varying vec3 vUv;
void main()
{
	vUv = uv;
	gl_Position = p;
}
