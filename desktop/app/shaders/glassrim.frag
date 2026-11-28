// SPDX-License-Identifier: GPL-3.0-or-later
// The edge of a Liquid Glass surface: a delicate outer contrast hairline, a crisp
// luminous top-left and top-edge specular catch with a soft bottom-right bounce,
// an inner edge-thickness glow, and a whisper-soft top-down surface reflection.
// Samples no textures. Output is premultiplied.
//
// Compiled to glassrim.frag.qsb with Qt's qsb (see shaders.qrc).
#version 440

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
    vec2 itemSize;
    float radius;
    float strength;
    float sheenStrength;
    vec4 rimColor;
    vec4 outerColor;
};

void main() {
    vec2 half_ = itemSize * 0.5;
    vec2 p = qt_TexCoord0 * itemSize - half_;
    float r = min(radius, min(half_.x, half_.y));
    vec2 q = abs(p) - half_ + r;
    // Signed distance to the rounded rectangle: negative inside.
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
    vec2 outward = sign(p) * max(q, 0.0);
    float len = length(outward);
    vec2 n = len > 0.0001 ? outward / len : vec2(0.0);

    // Mask strictly inside the rounded rectangle so corners stay crisp.
    float inside = 1.0 - smoothstep(-0.75, 0.25, d);

    // Delicate outer contrast hairline right at the boundary so the bright inner
    // glass rim pops cleanly against both bright and dark wallpapers.
    float outerBand = (1.0 - smoothstep(0.0, 0.85, abs(d + 0.42))) * inside;

    // Crisp luminous inner specular rim inset just inside the outer hairline,
    // plus a smooth inner thickness glow.
    float rim = 1.0 - smoothstep(0.0, 1.05, abs(d + 1.35));
    float innerEdge = 1.0 - smoothstep(-1.4, -0.5, d);
    float glow = exp(min(d + 0.5, 0.0) / 6.5) * innerEdge * inside;

    // Directional overhead-left specular catch: concentrated on top-left curved
    // corners and top bevel, with a soft subtle bottom-right bounce.
    float topLeftCorner = pow(clamp(dot(n, normalize(vec2(-0.68, -0.73))), 0.0, 1.0), 2.5);
    float topBevel = pow(clamp(-n.y, 0.0, 1.0), 2.4) * 0.62;
    float key = max(topLeftCorner, topBevel);
    float fill = pow(clamp(dot(n, normalize(vec2(0.60, 0.80))), 0.0, 1.0), 3.2) * 0.34;
    float light = key + fill;

    // Whisper-soft top-down inner glass reflection over the upper ~28% of the surface.
    float sheenBand = pow(smoothstep(0.28, 0.0, qt_TexCoord0.y), 1.35);
    float topSheen = sheenBand * inside * sheenStrength;

    float whiteA = clamp(
        ((rim * (0.15 + 0.85 * light) + glow * 0.24 * light) * strength + topSheen * (0.85 + 0.15 * strength))
        * rimColor.a * qt_Opacity,
        0.0,
        1.0
    );
    float outerA = clamp(outerBand * outerColor.a * qt_Opacity, 0.0, 1.0);

    // Composite outer contrast hairline over the bright specular rim in premultiplied alpha.
    float totalA = clamp(outerA + whiteA * (1.0 - outerA), 0.0, 1.0);
    vec3 totalRgb = outerColor.rgb * outerA + rimColor.rgb * whiteA * (1.0 - outerA);
    fragColor = vec4(totalRgb, totalA);
}
