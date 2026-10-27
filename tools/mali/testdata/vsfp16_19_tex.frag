#version 100
#extension GL_OES_standard_derivatives : enable
precision mediump float;
uniform sampler2D sTexture;
uniform lowp vec4 uScale;
varying lowp vec4 Tint;
varying mediump vec2 TexCoord;
void main(void)
{
	gl_FragColor = texture2D(sTexture, TexCoord) * Tint * uScale;
}
