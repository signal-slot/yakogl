const vec4 Albedo = vec4(0.0, 0.0, 1.0, 1.0);
const vec4 LightDir = vec4(0.0, 3.0, 2.0, 1.0);
attribute vec3 position;
attribute vec3 normal;
uniform mat4 ModelViewProjectionMatrix;
uniform mat4 NormalMatrix;
varying vec4 Color;
void main()
{
    vec3 N = normalize(vec3(NormalMatrix * vec4(normal, 1.0)));
    vec3 L = normalize(LightDir.xyz);
    float diffuse = max(dot(N, L), 0.0);
    Color = vec4(diffuse * Albedo.rgb, Albedo.a);
    gl_Position = ModelViewProjectionMatrix * vec4(position, 1.0);
}
