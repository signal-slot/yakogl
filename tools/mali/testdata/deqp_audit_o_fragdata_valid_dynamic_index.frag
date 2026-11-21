varying mediump vec4 v_color;
varying mediump float v_index;
uniform mediump int u_index;
void main (void)
{
    gl_FragData[int(v_index)] = v_color;
}
