precision mediump float;
varying vec4 vp;
varying vec3 vn;
const float Eta = 1.2;
void main()
{
    vec3 e = normalize(-vp.xyz);
    vec3 n = normalize(vn);
    vec3 r = refract(e, n, Eta);
    gl_FragColor = vec4(r * 0.5 + 0.5, 1.0);
}
