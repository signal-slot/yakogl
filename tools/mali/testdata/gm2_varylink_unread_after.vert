/* glmark2 pulsar shape (9d75+4748): two varyings from attributes, the
 * fragment shader reads only Color; the unread vec2 is laid out after it. */
attribute vec4 position;
attribute vec4 vtxcolor;
attribute vec2 texcoord;
uniform mat4 mvp;
varying vec4 Color;
varying vec2 TextureCoord;
void main()
{
	Color = vtxcolor;
	TextureCoord = texcoord;
	gl_Position = mvp * position;
}
