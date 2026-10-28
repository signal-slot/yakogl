#version 300 es
precision mediump float;
precision mediump sampler2DArray;
uniform sampler2DArray layers;
uniform float layer;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    vec2 uv = v_xy * 0.5 + 0.5;
    ivec3 sz = textureSize(layers, 0);
    vec4 t = texture(layers, vec3(uv, layer));
    vec4 f = texelFetch(layers, ivec3(int(uv.x * float(sz.x)), int(uv.y * float(sz.y)), int(layer)), 0);
    fragColor = vec4(t.rgb, f.a);
}
