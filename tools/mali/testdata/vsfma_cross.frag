precision mediump float;
varying vec3 v_b;
void main()
{
    gl_FragColor = vec4(v_b, 1.0);
}
