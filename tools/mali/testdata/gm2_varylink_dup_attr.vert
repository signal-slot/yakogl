/* glmark2 ideas table shape (bf4b+23c8): color = vec4(c, c, c, 1.0), c from
 * the attribute and a uniform-guarded scale: three equal lanes and a
 * constant one. */
attribute vec4 vertex;
uniform mat4 mvp;
uniform vec3 lightPosition;
uniform vec3 logoDirection;
uniform float currentTime;
varying vec4 color;
void main()
{
	gl_Position = mvp * vertex;
	vec3 ld = lightPosition - vertex.xyz;
	float c = dot(ld, logoDirection);
	c = c * c * ld.y;
	if (currentTime > 10.0)
		c *= 1.0 - (currentTime - 10.0) * 0.5;
	color = vec4(c, c, c, 1.0);
}
