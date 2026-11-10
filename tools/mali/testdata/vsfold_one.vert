attribute vec3 position;
attribute vec2 texcoord;
uniform mat4 ModelViewProjectionMatrix;
varying vec2 TextureCoord;
void main()
{
    TextureCoord = texcoord;
    gl_Position = ModelViewProjectionMatrix * vec4(position, 1.0);
}
