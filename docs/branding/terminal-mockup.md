# KELLU — Terminal identity mockup

![terminal mockup](mockups/kellu-terminal.png)

## Purpose

This is a visual/interaction reference, not a claim that the production CLI already implements every detail.

## Design principles

- Keep terminal art text-based rather than depending on inline image support.
- Offer Unicode and plain ASCII versions.
- Use monospace layout and test more than one terminal font.
- Apply ANSI colours only to interactive TTY output and respect `NO_COLOR`.
- Do not alter the user's terminal background or global palette.
- Ensure the descriptor and accents remain legible against the terminal's existing background.
- Do not rely on colour alone to distinguish the mark.

## Candidate layout

Unicode:
```text
  ◇
‹ ◈ › kellu
  ◇   Secure and Modular AI Assistant
```

ASCII fallback:
```text
  .
< * > kellu
  .   Secure and Modular AI Assistant
```

These are conceptual layouts. Use the tested local prototype for final spacing, glyph choice and alignment.

## Production checklist

- [ ] Test in the current terminal and another monospace font.
- [ ] Test with colour disabled and with `NO_COLOR` set.
- [ ] Test when stdout is piped instead of attached to a TTY.
- [ ] Ensure decorative output does not interfere with scripts or machine-readable output.
