#version 100
precision highp float;
varying vec3 v_dir;
void main()
{
    vec3 d = normalize(v_dir);
    vec2 uv = vec2(atan(d.z, d.x), asin(d.y)) * vec2(0.1591549, 0.3183099) + 0.5;
    gl_FragColor = vec4(uv, 0.0, 1.0);
}
