/* dEQP-GLES2.functional.light_amount, its vertex shader template with
 * NUM_DIR_LIGHTS = 0 and NUM_OMNI_LIGHTS = 16 (VK-GL-CTS
 * modules/gles2/functional/es2fLightAmountTest.cpp, Apache-2.0). Two
 * constant-bound loops over a uniform struct array: the unroller expands
 * them. At 16 lights it fitted only once constants fold and each uniform is
 * loaded at its first read (prepare.rs fold, select.rs Sel::val). */
struct Light
{
    vec3    position;
    vec3    diffuse;
    vec3    specular;
    vec3    attenuation;
};
uniform Light u_lights[0 + 16];
uniform mat4 u_modelviewMatrix;
uniform mat4 u_modelviewProjectionMatrix;
uniform mat3 u_normalMatrix;

attribute vec4 a_position;
attribute vec3 a_normal;

varying vec3 v_color;

float computeAttenuation(vec3 dirToLight, vec3 attenuation)
{
    float dist = length(dirToLight);
    return 1.0 / (attenuation.x + attenuation.y*dist + attenuation.z*dist*dist);
}

vec3 computeDirLight(int ndx, vec3 position, vec3 normal)
{
    Light light = u_lights[ndx];
    float cosAngle = dot(light.position, normal);
    return cosAngle * light.diffuse;
}

vec3 computeOmniLight(int ndx, vec3 position, vec3 normal)
{
    Light light = u_lights[ndx];
    vec3 dirToLight = light.position - position;
    float cosAngle = dot(normalize(dirToLight), normal);
    float atten = computeAttenuation(dirToLight, light.attenuation);
    return atten * cosAngle * light.diffuse;
}

void main()
{
    vec3 lightSpacePos = vec3(u_modelviewMatrix * a_position);
    vec3 lightNormal = normalize(u_normalMatrix * a_normal);
    vec3 color = vec3(0.0);
    for (int i = 0; i < 0; i++)
        color += computeDirLight(i, lightSpacePos, lightNormal);
    for (int i = 0; i < 16; i++)
        color += computeOmniLight(0+i, lightSpacePos, lightNormal);
    v_color = color;
    gl_Position = u_modelviewProjectionMatrix * a_position;
}
