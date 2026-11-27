varying mediump vec4 v_coords;
uniform mediump int ui_one;

void main ()
{
    gl_FragColor = vec4(v_coords.xyz, 1.0);
    if (false)
        return;
    gl_FragColor = vec4(v_coords.wzy, 1.0);
}
