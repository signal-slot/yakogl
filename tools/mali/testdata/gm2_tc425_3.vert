#version 100
/* ace2 with a uniform diffuse colour (three distinct lanes) and a constant alpha */
const vec4 LightSourcePosition = vec4(20.0, 20.0, 10.0, 1.0);
uniform vec4 MaterialDiffuse;
attribute vec3 position;
attribute vec3 normal;
attribute vec2 texcoord;

uniform mat4 ModelViewProjectionMatrix;
uniform mat4 NormalMatrix;

varying vec4 Color;
varying vec2 TextureCoord;

void main(void)
{
    vec3 N = normalize(vec3(NormalMatrix * vec4(normal, 1.0)));
    vec3 L = normalize(LightSourcePosition.xyz);
    float diffuse = max(dot(N, L), 0.0);
    Color = vec4(diffuse * MaterialDiffuse.rgb, 1.0);
    TextureCoord = texcoord;
    gl_Position = ModelViewProjectionMatrix * vec4(position, 1.0);
}
