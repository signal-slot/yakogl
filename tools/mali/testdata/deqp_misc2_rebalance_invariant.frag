uniform mediump vec4 u_color;
varying mediump vec4 v_unrelated;
void main ()
{
    mediump float blue = dot(v_unrelated, vec4(1.0, 1.0, 1.0, 1.0));
    gl_FragColor = vec4(u_color.r, u_color.g, blue, u_color.a);
}
