// gl_test case 97 "varyings15": every varying checked; green when all fifteen
// arrive, red otherwise with blue = (index of the first wrong one + 1) / 16.
precision mediump float;
varying vec4 v0;
varying vec4 v1;
varying vec4 v2;
varying vec4 v3;
varying vec4 v4;
varying vec4 v5;
varying vec4 v6;
varying vec4 v7;
varying vec4 v8;
varying vec4 v9;
varying vec4 v10;
varying vec4 v11;
varying vec4 v12;
varying vec4 v13;
varying vec4 v14;
bool near(vec4 a, vec4 b)
{
    return all(lessThan(abs(a - b), vec4(0.004)));
}
void main()
{
    float bad = 0.0;
    if (!near(v14, vec4(15.0, 31.0, 47.0, 63.0) / 64.0))
        bad = 15.0 / 16.0;
    if (!near(v13, vec4(14.0, 30.0, 46.0, 62.0) / 64.0))
        bad = 14.0 / 16.0;
    if (!near(v12, vec4(13.0, 29.0, 45.0, 61.0) / 64.0))
        bad = 13.0 / 16.0;
    if (!near(v11, vec4(12.0, 28.0, 44.0, 60.0) / 64.0))
        bad = 12.0 / 16.0;
    if (!near(v10, vec4(11.0, 27.0, 43.0, 59.0) / 64.0))
        bad = 11.0 / 16.0;
    if (!near(v9, vec4(10.0, 26.0, 42.0, 58.0) / 64.0))
        bad = 10.0 / 16.0;
    if (!near(v8, vec4(9.0, 25.0, 41.0, 57.0) / 64.0))
        bad = 9.0 / 16.0;
    if (!near(v7, vec4(8.0, 24.0, 40.0, 56.0) / 64.0))
        bad = 8.0 / 16.0;
    if (!near(v6, vec4(7.0, 23.0, 39.0, 55.0) / 64.0))
        bad = 7.0 / 16.0;
    if (!near(v5, vec4(6.0, 22.0, 38.0, 54.0) / 64.0))
        bad = 6.0 / 16.0;
    if (!near(v4, vec4(5.0, 21.0, 37.0, 53.0) / 64.0))
        bad = 5.0 / 16.0;
    if (!near(v3, vec4(4.0, 20.0, 36.0, 52.0) / 64.0))
        bad = 4.0 / 16.0;
    if (!near(v2, vec4(3.0, 19.0, 35.0, 51.0) / 64.0))
        bad = 3.0 / 16.0;
    if (!near(v1, vec4(2.0, 18.0, 34.0, 50.0) / 64.0))
        bad = 2.0 / 16.0;
    if (!near(v0, vec4(1.0, 17.0, 33.0, 49.0) / 64.0))
        bad = 1.0 / 16.0;
    gl_FragColor = bad == 0.0 ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, bad, 1.0);
}
