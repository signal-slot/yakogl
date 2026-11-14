/* glmark2 ideas text shape (a09e+23c8): color = vec4(c, c, c, 1.0), c from
 * a uniform alone, through an if: three equal lanes of no attribute and a
 * constant one. */
attribute vec4 vertex;
uniform mat4 mvp;
uniform float currentTime;
varying vec4 color;
void main()
{
	gl_Position = mvp * vertex;
	float c = 0.0;
	if (currentTime > 10.0)
		c = (currentTime - 10.0) * 0.5;
	color = vec4(c, c, c, 1.0);
}
