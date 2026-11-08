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
uniform vec2 extent;
uniform vec4 radii;
uniform vec4 mask_rect;
uniform vec2 direction;
uniform float blur_radius;
uniform float flip_y;
#ifdef DEBUG_FLAGS
uniform float tint;
#endif
float distance_box(vec2 p) {
    vec2 box_size = mask_rect.zw;
    float r = p.x < box_size.x*0.5 ? (p.y < box_size.y*0.5 ? radii.x : radii.w) : (p.y < box_size.y*0.5 ? radii.y : radii.z);
    vec2 q = abs(p-box_size*0.5)-box_size*0.5+r;
    return min(max(q.x,q.y),0.0)+length(max(q,0.0))-r;
}
void main() {
    vec4 color = vec4(0.0);
    float total = 0.0;
    if (blur_radius <= 0.0) {
        color = texture2D(tex,v_coords);
#ifdef NO_ALPHA
        color.a = 1.0;
#endif
        total = 1.0;
    } else for (int i = -8; i <= 8; i++) {
        float t = float(i)/8.0;
        float weight = exp(-4.5*t*t);
        vec2 sample_at = clamp(v_coords + direction*t*blur_radius/extent, vec2(0.5)/extent, vec2(1.0)-vec2(0.5)/extent);
        vec4 sample_color = texture2D(tex,sample_at);
#ifdef NO_ALPHA
        sample_color.a = 1.0;
#endif
        color += sample_color*weight;
        total += weight;
    }
    vec2 local = vec2(v_coords.x, mix(v_coords.y,1.0-v_coords.y,flip_y))*extent - mask_rect.xy;
    float coverage = 1.0-smoothstep(-0.5,0.5,distance_box(local));
    gl_FragColor = color/total * alpha * coverage;
}
