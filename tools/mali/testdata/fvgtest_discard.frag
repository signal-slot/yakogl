#version 100
precision highp float;
#define UNIFORMARRAY_SIZE 14
uniform vec4 frag[UNIFORMARRAY_SIZE];
#define innerCol frag[6]
#define scissorMat mat3(frag[0].xyz, frag[1].xyz, frag[2].xyz)
#define scissorExt frag[8].xy
#define scissorScale frag[8].zw
#define strokeMult frag[10].x
#define strokeThr frag[10].y
#define scissorRadius frag[12].w
varying vec2 ftcoord;
varying vec2 fpos;
float sdroundrect(vec2 pt, vec2 ext, float rad) {
    vec2 ext2 = ext - vec2(rad, rad);
    vec2 d = abs(pt) - ext2;
    return min(max(d.x, d.y), 0.0) + length(max(d, 0.0)) - rad;
}
float scissorMask(vec2 p) {
    if (scissorRadius > 0.0) {
        vec2 pt = (scissorMat * vec3(p, 1.0)).xy;
        float distance = sdroundrect(pt, scissorExt, scissorRadius);
        return clamp(0.5 - distance * min(scissorScale.x, scissorScale.y), 0.0, 1.0);
    }
    vec2 sc = (abs((scissorMat * vec3(p, 1.0)).xy) - scissorExt);
    sc = vec2(0.5, 0.5) - sc * scissorScale;
    return clamp(sc.x, 0.0, 1.0) * clamp(sc.y, 0.0, 1.0);
}
float strokeMask() {
    return min(1.0, (1.0 - abs(ftcoord.x * 2.0 - 1.0)) * strokeMult) * min(1.0, ftcoord.y);
}
void main(void) {
    float strokeAlpha = strokeMask();
    if (strokeAlpha < strokeThr) discard;
    vec4 result = innerCol;
    float scissor = scissorMask(fpos);
    result *= strokeAlpha * scissor;
    gl_FragColor = result;
}
