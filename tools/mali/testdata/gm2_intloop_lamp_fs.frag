#version 100
/* A lit fragment shader whose int counter indexes a local array of structs
 * (three lights whose positions are uniforms), summing three contributions:
 * a constant trip count whose counter is a subscript. */
precision mediump float;
struct Light {
    vec4 diffuse;
    vec4 position;
};
uniform vec4 u_light0;
uniform vec4 u_light1;
uniform vec4 u_light2;
varying vec3 v_normal;
varying vec4 v_pos;
void main()
{
    Light lights[3];
    lights[0] = Light(vec4(1.0, 1.0, 1.0, 1.0), u_light0);
    lights[1] = Light(vec4(0.3, 0.3, 0.5, 1.0), u_light1);
    lights[2] = Light(vec4(0.2, 0.2, 0.2, 1.0), u_light2);
    vec3 n = normalize(v_normal);
    vec4 sum = vec4(0.0);
    for (int i = 0; i < 3; i++) {
        vec3 l = normalize(lights[i].position.xyz - v_pos.xyz);
        sum += max(0.0, dot(n, l)) * lights[i].diffuse;
    }
    gl_FragColor = vec4(1.0, 0.2, 0.2, 1.0) * sum;
}
