/* dEQP-GLES2.functional.shaders.linkage.varying_4, rebuilt from VK-GL-CTS
 * 93bca018 (Apache-2.0): data/gles2/shaders/linkage.test:73-84, the fragment
 * source (it has no ${...} to specialize). */
varying mediump float var;
uniform bool u_false;
void main()
{
    if (u_false)
        gl_FragColor = vec4(var);
    else
        gl_FragColor = vec4(1.0);
}
