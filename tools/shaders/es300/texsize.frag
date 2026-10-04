#version 300 es
precision highp float;
precision highp int;
uniform sampler2D tex;
precision mediump sampler2DArray;
uniform sampler2DArray arr;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    ivec2 s = textureSize(tex, 0);
    ivec3 a = textureSize(arr, 0);
    /* width and height of the 2D texture, then the array's per-layer height
     * and layer count, each divided by 255 so a byte read gives the number */
    fragColor = vec4(float(s.x) / 255.0, float(s.y) / 255.0,
                     float(a.y) / 255.0, float(a.z) / 255.0);
}
