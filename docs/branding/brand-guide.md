# KELLU — Brand guide

## Brand signature

KELLU combines a custom wordmark with a geometric isotype. The identity is intended to remain restrained and legible across documentation, terminal interfaces, and application UI.

## Colour palette

Selected working palette: gold/copper Option A, with the requested deep navy.

| Role | HEX | RGB | Use |
|---|---|---|---|
| Deep navy | `#091C2D` | `9, 28, 45` | Primary dark background |
| Antique gold | `#A9823F` | `169, 130, 63` | Includes the two small terminal diamonds |
| Copper | `#C07852` | `192, 120, 82` | Warm accent |
| Ivory | `#F4F0E7` | `244, 240, 231` | Optional light neutral |

Check the actual SVG on white, ivory and navy before treating the palette as release-final. Gold and copper must remain distinguishable at actual display size, especially in the small terminal diamonds.

## Logo variants

Prepare three compositions in four treatments each:
1. Full logo (isotype + wordmark).
2. Isotype only.
3. Wordmark only.

Treatments: colour for dark backgrounds, colour for light backgrounds, monochrome black, and monochrome white/inverted. This gives 11 SVG masters. Use transparent backgrounds; the white variant should not contain a white background rectangle.

Suggested SVG filenames:

```text
assets/branding/svg/
├── kellu-logo-color-dark.svg
├── kellu-logo-color-light.svg
├── kellu-logo-mono-black.svg
├── kellu-logo-mono-white.svg
├── kellu-isotype-color-dark.svg
├── kellu-isotype-color-light.svg
├── kellu-isotype-mono-black.svg
├── kellu-isotype-mono-white.svg
├── kellu-wordmark-color.svg
├── kellu-wordmark-black.svg
└── kellu-wordmark-white.svg
```

## Small-size behaviour

The two small terminal diamonds are critical details. Rasterization can close negative spaces and merge colours.

- Test PNG icon exports at 16, 24, 36, 48, 64, 72 and 128 px at 100% scale.
- Check the silhouette in monochrome as well as colour.
- If needed, create optical variants for 16–24 px rather than distorting the master SVG.
- Document optical adjustments in `isotype-construction.md`.

## Terminal use

Use text-based terminal art, not embedded images. Apply ANSI colours only for TTY output and respect `NO_COLOR`. Do not change the user's terminal background/theme. Provide an ASCII fallback and test alignment in monospace fonts.

## Image placeholders

Replace these paths with your actual local PNG exports:
- `images/1_telar_mapuche.jpeg`
- `images/2_poncho_mapuche.jpeg`
- `images/1_telar_mapuche.jpeg`
- `images/kellu_color_palette.png`
- `images/isotype-geometric-construction.png`
- `images/final-isotype-variants.png`
- `images/base-fonts-comparison.png`
- `images/base-fonts-comparison.png`
- `mockups/kellu-terminal.png`

