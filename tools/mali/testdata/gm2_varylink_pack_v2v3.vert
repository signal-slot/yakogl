/* glmark2 bump height shape (270a+77c5): a vec2 then three vec3, all from
 * attributes and all read whole. */
attribute vec4 position;
attribute vec3 normal;
attribute vec3 tangent;
attribute vec2 texcoord;
uniform mat4 mvp;
varying vec2 TextureCoord;
varying vec3 NormalEye;
varying vec3 TangentEye;
varying vec3 BitangentEye;
void main()
{
	TextureCoord = texcoord;
	NormalEye = normal;
	TangentEye = tangent;
	BitangentEye = normal * tangent;
	gl_Position = mvp * position;
}
