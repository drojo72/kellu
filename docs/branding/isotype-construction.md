<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcet="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# KELLU — Isotype geometric construction

## Purpose

Document the construction method so the isotype can be reviewed, reproduced and adapted. The original form emerged partly through visual exploration; this document records the subsequent geometric systematisation.

## 1. Construction dimensions

The current construction notes specify squares rotated by 45 degrees. The dimensions below are the source-square sides and resulting diamond extents as recorded during the Inkscape process.

| Element | Source-square side | Result after 45° rotation | Purpose |
|---|---:|---:|---|
| Original/base square | 16.617 mm | 23.500 × 23.500 mm diamond | Defines initial outer geometry and terminal points |
| Secondary square | 11.314 mm | 16.000 × 16.000 mm diamond | Used with Boolean difference to create the frame |
| Central square | 4.243 mm | 6.000 × 6.000 mm diamond | Central diamond |
| Terminal squares | 2.828 mm each | 4.000 × 4.000 mm diamonds | Two small diamonds on the central vertical axis |

These values follow the diagonal relation for a square rotated 45°: the point-to-point extent is approximately the side length multiplied by √2. Treat them as the working specification and confirm actual object dimensions in the SVG.

## 2. Construction sequence

1. Draw a 16.617 mm square.
2. Rotate it by 45°. Its point-to-point width and height are approximately 23.500 mm.
3. Insert an 11.314 mm square, rotate it 45° (approximately 16 mm point-to-point), and use Boolean Difference to form the outer frame.
4. Insert a 4.243 mm square and rotate it 45° to create the 6 mm central diamond.
5. Insert a rectangle 5 mm wide × 24 mm high, centred on the construction axis. Apply Boolean Difference as required, taking care not to cut the central diamond.
6. Insert two 2.828 mm squares, rotate each by 45°, and place them on the central vertical axis: one at the upper edge and one at the lower edge. Each forms a 4 × 4 mm diamond.
7. Trim the upper tips of the angular forms created by the Boolean operations. This adds breathing room around the small diamonds and softens the overall visual balance.
8. Apply the selected colours and inspect the full-colour result.

![Construction detailed](images/icono_estructura.png)

## 3. Boolean operations and layers

The exact object order matters. In the Inkscape construction plate, show each operation separately and label the operands before and after each difference. Keep construction geometry on a separate layer from the final filled paths.

The centred 5 × 24 mm rectangle is a cutting tool; its Boolean operation should preserve the central diamond. Record the actual sequence and object selections in the construction plate so another designer can reproduce it.

## 4. Measurements and final checks

- [ ] Confirm all dimensions in millimetres in Inkscape.
- [ ] Confirm rotation centres and alignment to the central axis.
- [ ] Record exact Boolean operation order.
- [ ] Confirm the 5 × 24 mm rectangle does not alter the central diamond.
- [ ] Confirm terminal diamonds have enough negative space after trimming.
- [ ] Check whether final geometry uses filled paths or strokes.
- [ ] Record the final SVG `viewBox` and export bounds.
- [ ] Export the master and compare it with the original visual prototype.

## 5. Small-size optical corrections

The terminal diamonds may merge visually with adjacent coloured geometry at 16–24 px. Test at 100% display size on light and dark backgrounds.

If necessary, increase negative space first, then test a small reduction in terminal-diamond size or a weight adjustment. Compare full colour and monochrome. Keep the master SVG as the source of truth and document any optical variant separately.

## 6. Important distinction

The geometric plate formalises the design after visual exploration. It should not imply that every proportion was mathematically specified in the first sketch.
