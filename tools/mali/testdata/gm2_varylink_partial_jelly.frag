precision mediump float;
uniform sampler2D uSampler;
varying vec2 vTextureCoord;
varying vec4 vWorld;
varying vec3 vDiffuse;
varying vec3 vAmbient;
varying vec3 vFresnel;
void main()
{
	vec4 colorMap = texture2D(uSampler, vTextureCoord);
	gl_FragColor = vec4((vAmbient + vDiffuse + vWorld.xyz) * colorMap.rgb, colorMap.a + vFresnel.r);
}
