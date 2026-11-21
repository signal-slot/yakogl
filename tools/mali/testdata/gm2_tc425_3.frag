#version 100
/* glmark2 texture scene, fs 425b: light-basic-tex.frag, glmark2's mediump default */
precision mediump float;
uniform sampler2D MaterialTexture0;

varying vec4 Color;
varying vec2 TextureCoord;

void main(void)
{
    vec4 texel = texture2D(MaterialTexture0, TextureCoord);
    gl_FragColor = texel * Color;
}
