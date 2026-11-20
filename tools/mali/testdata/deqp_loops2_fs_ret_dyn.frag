varying mediump vec4 v_coords;
uniform mediump int ui_one;

mediump vec4 getCoords (void)
{
    mediump vec4 coords = v_coords;
    for (int i = 0; i < ui_one; i++)
    {
        if (v_coords.x+v_coords.y >= 0.0)
            return coords;
        coords = coords.wzyx;
    }
    return coords;
}

void main (void)
{
    gl_FragColor = vec4(getCoords().xyz, 1.0);
}
