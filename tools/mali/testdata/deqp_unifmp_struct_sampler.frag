varying mediump vec4 v_coords;
uniform int ui_one;

struct S {
    mediump float    a;
    mediump vec3    b;
    sampler2D        c;
};
uniform S s;

void main (void)
{
    gl_FragColor = vec4(texture2D(s.c, v_coords.xy * s.b.xy + s.b.z).rgb, s.a);
    
}
