attribute vec3 pos;
attribute vec3 nrm;
uniform mat4 world;
uniform mat4 mvp;
uniform vec4 fcol;
uniform float fpow;
varying vec3 vf;
void main()
{
    vec4 w = world * vec4(pos, 1.0);
    vec3 n = normalize(nrm);
    vec3 eye = normalize(w.xyz / w.w);
    float f = (1.0 - max(abs(dot(n, -eye)), 0.0)) * fpow;
    vf = fcol.rgb * vec3(fcol.a * f);
    gl_Position = mvp * vec4(pos, 1.0);
}
