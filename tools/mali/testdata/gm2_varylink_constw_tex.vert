/* glmark2 texture shape (ace2+425b): constw_dup's vertex shader with a
 * fragment shader that reads both varyings, TextureCoord as a look-up's
 * coordinate. */
attribute vec4 position;
attribute vec3 normal;
attribute vec2 texcoord;
uniform mat4 mvp;
varying vec4 Color;
varying vec2 TextureCoord;
const vec4 MaterialDiffuse = vec4(1.0, 1.0, 1.0, 1.0);
const vec3 L = vec3(0.6, 0.0, 0.8);
void main()
{
	float diffuse = dot(normal, L);
	Color = vec4(diffuse * MaterialDiffuse.rgb, MaterialDiffuse.a);
	TextureCoord = texcoord;
	gl_Position = mvp * position;
}
