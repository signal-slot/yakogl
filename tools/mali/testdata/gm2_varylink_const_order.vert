/* glmark2 shadow shape (b9d1+cf4b): the vertex shader declares ShadowCoord
 * then Color, the fragment shader Color then ShadowCoord; Color is a whole
 * constant (three equal lanes) of which the colour reads rgb. */
attribute vec4 position;
uniform mat4 LightMatrix;
uniform mat4 mvp;
varying vec4 ShadowCoord;
varying vec4 Color;
const vec4 MaterialDiffuse = vec4(0.3, 0.3, 0.3, 1.0);
void main()
{
	Color = MaterialDiffuse;
	ShadowCoord = LightMatrix * position;
	gl_Position = mvp * position;
}
