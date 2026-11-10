precision mediump float;
varying vec4 Color;
varying vec4 ShadowCoord;
void main()
{
    gl_FragColor = Color + ShadowCoord * 0.01;
}
