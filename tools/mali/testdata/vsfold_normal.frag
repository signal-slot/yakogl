precision mediump float;
varying vec3 Normal;
void main()
{
    gl_FragColor = vec4(Normal * 0.5 + 0.5, 1.0);
}
