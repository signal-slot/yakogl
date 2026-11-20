precision mediump int;
uniform int ui_four;
varying mediump vec4 var[4];

void main()
{
    mediump vec4 res = vec4(0.0);
    for (int i = 0; i < ui_four; i++)
        res += var[i];
    gl_FragColor = vec4(res);
}
