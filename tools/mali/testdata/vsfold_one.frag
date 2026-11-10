precision mediump float;
varying vec2 TextureCoord;
void main()
{
    gl_FragColor = vec4(TextureCoord, 0.0, 1.0);
}
