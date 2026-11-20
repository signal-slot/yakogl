precision mediump int;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
varying mediump vec4 var[4];

void main()
{
    mediump vec4 res = vec4(0.0);
    res += var[ui_zero];
    res += var[ui_one];
    res += var[ui_two];
    res += var[ui_three];
    gl_FragColor = vec4(res);
}
