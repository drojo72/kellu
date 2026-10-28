<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcet="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# KELLU terminal mockups

Two small scripts to preview the proposed terminal header.

## Run

```bash
python3 kellu_ansi.py
python3 kellu_ansi.py --no-color
python3 kellu_ansi.py --no-bold
python3 kellu_ansi.py --plain-frame
python3 kellu_ascii.py
```

- `kellu_ansi.py`: Unicode symbol and optional ANSI styling. It respects `NO_COLOR` and avoids ANSI styling when stdout is not a TTY.
- `kellu_ascii.py`: strict ASCII-only, monochrome reference.

These are prototypes for visual adjustment, not final terminal-capability detection. The exact appearance of Unicode glyphs depends on the terminal font.
