#version 100
/* 425b with the varyings declared in the other order */
precision mediump float;
uniform sampler2D MaterialTexture0;

varying vec2 TextureCoord;
varying vec4 Color;

void main(void)
{
    vec4 texel = texture2D(MaterialTexture0, TextureCoord);
    gl_FragColor = texel * Color;
}
