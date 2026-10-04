#version 100

struct buf
{
    mat4 qt_Matrix;
    float opacity;
    vec2 pixelSize;
};

uniform buf _14;

attribute vec4 vertex;
varying vec2 texCoord;
attribute vec2 multiTexCoord;
attribute vec2 vertexOffset;
attribute vec2 texCoordOffset;
varying float vertexOpacity;

void main()
{
    vec4 pos = _14.qt_Matrix * vertex;
    vec4 m0 = _14.qt_Matrix[0];
    vec4 m1 = _14.qt_Matrix[1];
    gl_Position = pos;
    texCoord = multiTexCoord;
    if (vertexOffset.x != 0.0)
    {
        vec4 delta = m0 * vertexOffset.x;
        vec2 dir = (delta.xy * pos.w) - (pos.xy * delta.w);
        vec2 ndir = (_14.pixelSize * 0.5) * normalize(dir / _14.pixelSize);
        dir -= ((ndir * delta.w) * pos.w);
        float numerator = dot(dir, (ndir * pos.w) * pos.w);
        float scale = 0.0;
        if (numerator < 0.0)
        {
            scale = 1.0;
        }
        else
        {
            scale = min(1.0, numerator / dot(dir, dir));
        }
        gl_Position += (delta * scale);
        texCoord.x += (scale * texCoordOffset.x);
    }
    if (vertexOffset.y != 0.0)
    {
        vec4 delta_1 = m1 * vertexOffset.y;
        vec2 dir_1 = (delta_1.xy * pos.w) - (pos.xy * delta_1.w);
        vec2 ndir_1 = (_14.pixelSize * 0.5) * normalize(dir_1 / _14.pixelSize);
        dir_1 -= ((ndir_1 * delta_1.w) * pos.w);
        float numerator_1 = dot(dir_1, (ndir_1 * pos.w) * pos.w);
        float scale_1 = 0.0;
        if (numerator_1 < 0.0)
        {
            scale_1 = 1.0;
        }
        else
        {
            scale_1 = min(1.0, numerator_1 / dot(dir_1, dir_1));
        }
        gl_Position += (delta_1 * scale_1);
        texCoord.y += (scale_1 * texCoordOffset.y);
    }
    bool onEdge = any(notEqual(vertexOffset, vec2(0.0)));
    bool outerEdge = all(equal(texCoordOffset, vec2(0.0)));
    if (onEdge && outerEdge)
    {
        vertexOpacity = 0.0;
    }
    else
    {
        vertexOpacity = _14.opacity;
    }
}

