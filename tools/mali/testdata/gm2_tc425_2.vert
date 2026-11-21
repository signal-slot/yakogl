#version 100
/* glmark2 texture scene, vs ace2: light-basic.vert with the scene's constants */
const vec4 LightSourcePosition = vec4(20.0, 20.0, 10.0, 1.0);
const vec4 MaterialDiffuse = vec4(1.0, 1.0, 1.0, 1.0);
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
    Color = vec4(diffuse * MaterialDiffuse.rgb, MaterialDiffuse.a);
    TextureCoord = texcoord;
    gl_Position = ModelViewProjectionMatrix * vec4(position, 1.0);
}
