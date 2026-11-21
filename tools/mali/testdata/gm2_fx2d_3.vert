/* VS of gm2_fx2d_3.frag: the full-screen quad's coordinate (glmark2 effect-2d.vert
 * shape, original text). */
attribute vec3 position;
varying vec2 TextureCoord;
void main()
{
	gl_Position = vec4(position, 1.0);
	TextureCoord = position.xy * 0.5 + 0.5;
}
