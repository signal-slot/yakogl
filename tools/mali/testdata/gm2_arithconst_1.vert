const vec4 Tint = vec4(0.3, 0.3, 0.3, 1.0);
attribute vec2 position;
uniform mat4 LightMatrix;
uniform mat4 ModelViewProjectionMatrix;
varying vec4 Color;
varying vec4 ShadowCoord;
void main()
{
    Color = Tint;
    vec4 pos4 = vec4(position, 0.0, 1.0);
    ShadowCoord = LightMatrix * pos4;
    gl_Position = ModelViewProjectionMatrix * pos4;
}
