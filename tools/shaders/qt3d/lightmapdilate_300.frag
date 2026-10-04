#version 300 es
precision highp float;
precision highp int;

uniform highp sampler2D tex;

in vec2 v_uv;
layout(location = 0) out vec4 fragColor;

void main()
{
    vec2 uv = v_uv;
    vec2 sz = vec2(textureSize(tex, 0));
    vec2 pix = vec2(1.0 / sz.x, 1.0 / sz.y);
    vec4 c = texture(tex, uv);
    vec4 _50;
    if (c.w > 0.5)
    {
        _50 = c;
    }
    else
    {
        _50 = texture(tex, uv + vec2(-pix.x, 0.0));
    }
    c = _50;
    vec4 _68;
    if (c.w > 0.5)
    {
        _68 = c;
    }
    else
    {
        _68 = texture(tex, uv + vec2(0.0, pix.y));
    }
    c = _68;
    vec4 _84;
    if (c.w > 0.5)
    {
        _84 = c;
    }
    else
    {
        _84 = texture(tex, uv + vec2(pix.x, 0.0));
    }
    c = _84;
    vec4 _100;
    if (c.w > 0.5)
    {
        _100 = c;
    }
    else
    {
        _100 = texture(tex, uv + vec2(0.0, -pix.y));
    }
    c = _100;
    vec4 _117;
    if (c.w > 0.5)
    {
        _117 = c;
    }
    else
    {
        _117 = texture(tex, uv + vec2(-pix.x, -pix.y));
    }
    c = _117;
    vec4 _137;
    if (c.w > 0.5)
    {
        _137 = c;
    }
    else
    {
        _137 = texture(tex, uv + vec2(-pix.x, pix.y));
    }
    c = _137;
    vec4 _156;
    if (c.w > 0.5)
    {
        _156 = c;
    }
    else
    {
        _156 = texture(tex, uv + vec2(pix.x, -pix.y));
    }
    c = _156;
    vec4 _175;
    if (c.w > 0.5)
    {
        _175 = c;
    }
    else
    {
        _175 = texture(tex, uv + vec2(pix.x, pix.y));
    }
    c = _175;
    vec4 _193;
    if (c.w > 0.5)
    {
        _193 = c;
    }
    else
    {
        _193 = texture(tex, uv + vec2((-2.0) * pix.x, 0.0));
    }
    c = _193;
    vec4 _211;
    if (c.w > 0.5)
    {
        _211 = c;
    }
    else
    {
        _211 = texture(tex, uv + vec2(0.0, 2.0 * pix.y));
    }
    c = _211;
    vec4 _229;
    if (c.w > 0.5)
    {
        _229 = c;
    }
    else
    {
        _229 = texture(tex, uv + vec2(2.0 * pix.x, 0.0));
    }
    c = _229;
    vec4 _246;
    if (c.w > 0.5)
    {
        _246 = c;
    }
    else
    {
        _246 = texture(tex, uv + vec2(0.0, (-2.0) * pix.y));
    }
    c = _246;
    vec4 _263;
    if (c.w > 0.5)
    {
        _263 = c;
    }
    else
    {
        _263 = texture(tex, uv + vec2((-2.0) * pix.x, -pix.y));
    }
    c = _263;
    vec4 _283;
    if (c.w > 0.5)
    {
        _283 = c;
    }
    else
    {
        _283 = texture(tex, uv + vec2((-2.0) * pix.x, pix.y));
    }
    c = _283;
    vec4 _302;
    if (c.w > 0.5)
    {
        _302 = c;
    }
    else
    {
        _302 = texture(tex, uv + vec2(2.0 * pix.x, -pix.y));
    }
    c = _302;
    vec4 _322;
    if (c.w > 0.5)
    {
        _322 = c;
    }
    else
    {
        _322 = texture(tex, uv + vec2(2.0 * pix.x, pix.y));
    }
    c = _322;
    vec4 _341;
    if (c.w > 0.5)
    {
        _341 = c;
    }
    else
    {
        _341 = texture(tex, uv + vec2(-pix.x, (-2.0) * pix.y));
    }
    c = _341;
    vec4 _361;
    if (c.w > 0.5)
    {
        _361 = c;
    }
    else
    {
        _361 = texture(tex, uv + vec2(-pix.x, 2.0 * pix.y));
    }
    c = _361;
    vec4 _381;
    if (c.w > 0.5)
    {
        _381 = c;
    }
    else
    {
        _381 = texture(tex, uv + vec2(pix.x, (-2.0) * pix.y));
    }
    c = _381;
    vec4 _400;
    if (c.w > 0.5)
    {
        _400 = c;
    }
    else
    {
        _400 = texture(tex, uv + vec2(pix.x, 2.0 * pix.y));
    }
    c = _400;
    vec4 _419;
    if (c.w > 0.5)
    {
        _419 = c;
    }
    else
    {
        _419 = texture(tex, uv + vec2((-2.0) * pix.x, (-2.0) * pix.y));
    }
    c = _419;
    vec4 _439;
    if (c.w > 0.5)
    {
        _439 = c;
    }
    else
    {
        _439 = texture(tex, uv + vec2((-2.0) * pix.x, 2.0 * pix.y));
    }
    c = _439;
    vec4 _459;
    if (c.w > 0.5)
    {
        _459 = c;
    }
    else
    {
        _459 = texture(tex, uv + vec2(2.0 * pix.x, (-2.0) * pix.y));
    }
    c = _459;
    vec4 _479;
    if (c.w > 0.5)
    {
        _479 = c;
    }
    else
    {
        _479 = texture(tex, uv + vec2(2.0 * pix.x, 2.0 * pix.y));
    }
    c = _479;
    fragColor = c;
}

