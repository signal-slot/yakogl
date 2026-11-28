precision mediump int;
varying mediump vec2 var[4];

void main()
{
    mediump vec2 res = vec2(0.0);
    res += var[0];
    res += var[1];
    res += var[2];
    res += var[3];
    gl_FragColor = vec4(res, 0.0, 1.0);
}
