attribute vec2 position;
uniform mat4 ModelViewProjectionMatrix;
uniform mat4 LightMatrix;
uniform vec4 MaterialDiffuse;
varying vec4 Color;
varying vec4 ShadowCoord;
void main()
{
    Color = MaterialDiffuse;
    vec4 pos4 = vec4(position, 0.0, 1.0);
    ShadowCoord = LightMatrix * pos4;
    gl_Position = ModelViewProjectionMatrix * pos4;
}
