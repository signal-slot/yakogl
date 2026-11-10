attribute vec3 position;
attribute vec3 normal;
uniform mat4 ModelViewProjectionMatrix;
uniform mat4 NormalMatrix;
varying vec3 Normal;
void main()
{
    Normal = vec3(NormalMatrix * vec4(normal, 1.0));
    gl_Position = ModelViewProjectionMatrix * vec4(position, 1.0);
}
