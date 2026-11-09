// gl_test case 101 "cubebias": the vertex shader's texel, as it arrives.
precision mediump float;
varying vec4 v_col;
void main()
{
    gl_FragColor = v_col;
}
