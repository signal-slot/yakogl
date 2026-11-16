varying mediump vec4 v_coords;

mediump vec4 getColor (void)
{
    if (true)
        return vec4(v_coords.xyz, 1.0);
    return vec4(v_coords.wzy, 1.0);
}

void main (void)
{
    gl_FragColor = getColor();
}
