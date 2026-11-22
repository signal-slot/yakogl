precision highp float;
attribute highp vec4 dEQP_Position;
varying float out0;


struct S
{
    vec4 v;
    int  i;
};

struct T
{
    S s;
    bool b;
    int i;
};

struct U
{
    S s;
    T t;
};

void main()
{
    const S s = S(vec4(1.5), 123);
    const T t = T(s, false, 3);
    const U u = U(s, t);
    const U v = U(S(vec4(1.3), 4), T(S(vec4(2.0), 5), true, 6));
    out0 = float(u.s.i*v.t.i + v.t.s.i)*v.s.v.x; // float(123*6 + 5)*1.3
    gl_Position = dEQP_Position;

}
