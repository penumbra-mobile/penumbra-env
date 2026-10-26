#version 100
precision mediump float;

varying vec2 v_coords;
uniform sampler2D u_scene_tex;  // Sharp scene texture (unit 0)
uniform sampler2D u_blur_tex;   // Diffuse blurred texture (unit 1)
uniform float u_progress;       // Gesture progress t in [0.0, 1.0]

// # Mathematical Formulation
// Official Android 15/16/17 Material 3 Depth & Color Diffusion Model:
// 1. Color Saturation Modulation:
//    As blur intensity increases, colors diffuse and are boosted in saturation
//    to prevent perceptual desaturation from low-pass filtering.
//    sat_boost = 1.0 + t * 0.42
// 2. Light & Chromatic Diffusion Bloom:
//    Luminous and saturated color elements bleed softly outward into surrounding glass.
//    C_glow = max(C_sat - 0.12, vec3(0.0))
//    C_diffused = C_sat + C_glow * (0.30 * t)
// 3. Progressive Hermite Cross-Fade:
//    w(t) = smoothstep(0.0, 0.75, t)
//    C_scene = mix(C_sharp, C_diffused, w(t))
// 4. Material 3 Deep Obsidian Scrim (#090810):
//    alpha_scrim = smoothstep(0.0, 0.85, t) * 0.45
//    C_final = mix(C_scene, C_scrim, alpha_scrim)

void main() {
    float t = clamp(u_progress, 0.0, 1.0);
    float blend_factor = smoothstep(0.0, 0.75, t);

    vec4 sharp_col = texture2D(u_scene_tex, v_coords);
    vec4 blur_col = texture2D(u_blur_tex, v_coords);

    // 1. Luminance and saturation boost with blur intensity
    float luma = dot(blur_col.rgb, vec3(0.2126, 0.7152, 0.0722));
    float sat_boost = 1.0 + t * 0.42;
    vec3 saturated_col = mix(vec3(luma), blur_col.rgb, sat_boost);

    // 2. Color diffusion bloom: light bleed from saturated highlights
    vec3 diffuse_glow = max(saturated_col - 0.12, vec3(0.0));
    vec3 diffused_col = saturated_col + diffuse_glow * (0.30 * t);

    // 3. Cross-fade between sharp scene and diffused blur
    vec3 scene_col = mix(sharp_col.rgb, diffused_col, blend_factor);

    // 4. Deep obsidian scrim for contrast
    vec3 scrim_col = vec3(0.035, 0.032, 0.048);
    float scrim_alpha = smoothstep(0.0, 0.85, t) * 0.45;
    vec3 final_rgb = mix(scene_col, scrim_col, scrim_alpha);

    gl_FragColor = vec4(final_rgb, 1.0);
}
