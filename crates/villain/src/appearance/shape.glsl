precision highp float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
uniform vec4 radii;
uniform vec4 widths;
uniform vec4 top_color;
uniform vec4 right_color;
uniform vec4 bottom_color;
uniform vec4 left_color;
uniform vec4 body;
uniform vec4 shadow_data;
uniform vec4 shadow_color;
uniform float side;
#ifdef DEBUG_FLAGS
uniform float tint;
#endif
float distance_box(vec2 p, vec2 extent, vec4 r) {
    float radius = p.x < extent.x * 0.5 ? (p.y < extent.y * 0.5 ? r.x : r.w) : (p.y < extent.y * 0.5 ? r.y : r.z);
    vec2 q = abs(p - extent * 0.5) - extent * 0.5 + radius;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - radius;
}
void main() {
    vec2 p = v_coords * size - body.xy;
    float d = distance_box(p, body.zw, radii);
    if (side >= 0.0) {
        // Partition by nearest box edge. Side settings do not bleed onto opposite edges.
        vec4 edge = vec4(-p.y, p.x-body.z, p.y-body.w, -p.x);
        float selected = side < 0.5 ? edge.x : side < 1.5 ? edge.y : side < 2.5 ? edge.z : edge.w;
        if (selected < max(max(edge.x, edge.y), max(edge.z, edge.w)) || d < 0.0) { gl_FragColor = vec4(0.0); return; }
        float shifted = distance_box(p - shadow_data.xy, body.zw, radii) - shadow_data.w;
        float sigma = max(shadow_data.z, 0.25);
        float strength = exp(-0.5 * pow(max(shifted, 0.0) / sigma, 2.0));
        if (shifted > sigma * 4.0) strength = 0.0;
        float a = shadow_color.a * strength * alpha;
        gl_FragColor = vec4(shadow_color.rgb * a, a);
        return;
    }
    float outer = 1.0 - smoothstep(-0.5, 0.5, d);
    vec2 inner_p = p - vec2(widths.w, widths.x);
    vec2 inner_size = max(body.zw - vec2(widths.w + widths.y, widths.x + widths.z), vec2(1.0));
    vec4 inner_r = max(radii - vec4(max(widths.x,widths.w),max(widths.x,widths.y),max(widths.z,widths.y),max(widths.z,widths.w)),vec4(0.0));
    float inner = 1.0 - smoothstep(-0.5, 0.5, distance_box(inner_p, inner_size, inner_r));
    vec4 ratios = vec4(p.y / max(widths.x, 0.001), (body.z-p.x)/max(widths.y,0.001), (body.w-p.y)/max(widths.z,0.001), p.x/max(widths.w,0.001));
    float near = min(min(ratios.x,ratios.y),min(ratios.z,ratios.w));
    vec4 color = near == ratios.x ? top_color : near == ratios.y ? right_color : near == ratios.z ? bottom_color : left_color;
    float a = color.a * outer * (1.0-inner) * alpha;
    gl_FragColor = vec4(color.rgb*a,a);
}
