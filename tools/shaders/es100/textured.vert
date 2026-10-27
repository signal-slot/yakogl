#version 100
attribute highp vec4 inVertex;
attribute mediump vec2 inTexCoord;
attribute lowp vec4 inTint;
uniform mediump mat4 MVPMatrix;
varying mediump vec2 TexCoord;
varying lowp vec4 Tint;
varying mediump float Unused;
void main(void)
{
	gl_Position = MVPMatrix * inVertex;
	TexCoord = inTexCoord;
	Tint = inTint;
	Unused = 0.5;
}
