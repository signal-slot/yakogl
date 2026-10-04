// gl_test case 101 "cubebias": textureCube with a bias, the GC7000's texldb.
precision mediump float;
uniform samplerCube u_s;
uniform float u_bias;
varying vec3 v_dir;
void main()
{
    gl_FragColor = textureCube(u_s, v_dir, u_bias);
}
