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
