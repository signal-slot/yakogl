// A swizzled read of a local holding a BROADCAST literal.
//
// `vec4(0.5)` puts the same value in every lane, so no permutation of
// c.yzxw can change the picture -- which is exactly why this shape was
// invisible in every other source in the corpus. gles2-core's selection
// carried an immediate's payload bits in the swizzle/neg/abs fields and
// permuted them like lanes, turning 0.5 into 0.00036621094 (their c7eb385).
//
// It only reached that code through a STORE to a local slot and a swizzled
// LOAD back, which is how the front end lowers a named constructor
// variable; a direct `vec4(0.5).yzxw` never got that far. The uniform keeps
// the whole expression from folding away before it is emitted.
precision mediump float;
uniform float u_scale;
void main()
{
	vec4 c = vec4(0.5);
	gl_FragColor = c.yzxw * u_scale;
}
