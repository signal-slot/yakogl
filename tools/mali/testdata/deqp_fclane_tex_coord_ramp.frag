varying mediump vec2 v_coord;
void main (void)
{
    mediump float x = v_coord.x;
    mediump float y = v_coord.y;
    mediump float f0 = (x + y) * 0.5;
    mediump float f1 = 0.5 + (x - y) * 0.5;
    gl_FragColor = vec4(f0, f1, 1.0-f0, 1.0-f1);
}
