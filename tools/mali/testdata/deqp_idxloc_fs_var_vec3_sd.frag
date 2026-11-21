precision mediump int;
uniform int ui_four;
varying mediump vec3 var[4];

void main()
{
    mediump vec3 res = vec3(0.0);
    for (int i = 0; i < ui_four; i++)
        res += var[i];
    gl_FragColor = vec4(res, 1.0);
}
