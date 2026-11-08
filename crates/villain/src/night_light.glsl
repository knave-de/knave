#version 100
//_DEFINES_
#ifdef EXTERNAL
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#ifdef EXTERNAL
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
varying vec2 v_coords;
uniform float alpha;
uniform vec3 gains;
void main() {
    vec4 color=texture2D(tex,v_coords);
#ifdef NO_ALPHA
    color.a=1.0;
#endif
    gl_FragColor=vec4(color.rgb*gains,color.a)*alpha;
}
