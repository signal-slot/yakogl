precision mediump float;
varying vec4 vp;
varying vec3 vn;
void main()
{
    vec3 e = normalize(-vp.xyz);
    gl_FragColor = vec4(e, 1.0);
}
