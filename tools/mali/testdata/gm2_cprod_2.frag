precision mediump float;
varying vec4 vp;
varying vec3 vn;
void main()
{
    vec3 e = normalize(-vp.xyz);
    vec3 n = normalize(vn);
    vec3 l = normalize(vec3(0.0, 1.0, 0.0) - vp.xyz / vp.w);
    vec3 r = reflect(-l, n);
    float s = pow(max(0.0, dot(r, e)), 100.0);
    gl_FragColor = vec4(vec3(s), 1.0);
}
