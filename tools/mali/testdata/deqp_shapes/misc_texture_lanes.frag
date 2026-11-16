varying mediump vec4 v_coords;
uniform int ui_one;

struct S {
    mediump float    a;
    mediump vec3    b;
    sampler2D        c;
};
uniform S s[2];

void main (void)
{
    gl_FragColor = vec4(texture2D(s[1].c, v_coords.xy * s[0].b.xy + s[1].b.z).rgb, s[0].a);
    
}
