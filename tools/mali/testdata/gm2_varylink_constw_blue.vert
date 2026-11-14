/* glmark2 shading gouraud shape (83cd+4748): Color is a lit term times a
 * constant blue material, vec4(0.0, 0.0, d, 1.0) after the folds (three
 * constant lanes); TextureCoord is not read. */
attribute vec4 position;
attribute vec3 normal;
attribute vec2 texcoord;
uniform mat4 mvp;
varying vec4 Color;
varying vec2 TextureCoord;
const vec4 MaterialDiffuse = vec4(0.0, 0.0, 1.0, 1.0);
const vec3 L = vec3(0.6, 0.0, 0.8);
void main()
{
	float diffuse = dot(normal, L);
	Color = vec4(diffuse * MaterialDiffuse.rgb, MaterialDiffuse.a);
	TextureCoord = texcoord;
	gl_Position = mvp * position;
}
