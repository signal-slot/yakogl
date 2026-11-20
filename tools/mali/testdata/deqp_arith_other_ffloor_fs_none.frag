varying mediump vec4 v_coords;
void main (void)
{
	bool isOk = gl_MaxCombinedTextureImageUnits == (32 + int(floor(v_coords.x * 4.0) + 0.05));
	gl_FragColor = isOk ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0);
}
