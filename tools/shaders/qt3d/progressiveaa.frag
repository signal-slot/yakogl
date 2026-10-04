#version 100
precision highp float;
precision highp int;

struct buf
{
    vec2 blend;
};

uniform buf ubuf;

uniform highp sampler2D tex1;
uniform highp sampler2D tex2;

varying vec2 uv_coord;

void main()
{
    vec4 t1 = texture2D(tex1, uv_coord);
    vec4 t2 = texture2D(tex2, uv_coord);
    gl_FragData[0] = (t1 * ubuf.blend.x) + (t2 * ubuf.blend.y);
}

