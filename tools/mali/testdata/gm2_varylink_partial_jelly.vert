/* glmark2 jellyfish shape (08a2+22fc): five varyings from attributes; the
 * fragment shader reads vWorld.xyz and vFresnel.x only. */
attribute vec3 aVertexPosition;
attribute vec3 aVertexNormal;
attribute vec3 aVertexColor;
attribute vec3 aTextureCoord;
uniform mat4 uWorld;
uniform mat4 uWorldViewProj;
varying vec2 vTextureCoord;
varying vec4 vWorld;
varying vec3 vDiffuse;
varying vec3 vAmbient;
varying vec3 vFresnel;
void main()
{
	vec4 pos4 = vec4(aVertexPosition, 1.0);
	vWorld = uWorld * pos4;
	vDiffuse = aVertexColor * aVertexNormal.x;
	vAmbient = aVertexNormal * aVertexNormal.y;
	vFresnel = aVertexColor * aVertexNormal.z;
	vTextureCoord = aTextureCoord.xy;
	gl_Position = uWorldViewProj * pos4;
}
