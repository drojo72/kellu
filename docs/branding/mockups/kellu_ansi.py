#!/usr/bin/env python3
"""KELLU terminal header mockup with ANSI styling.

Run:
  python3 kellu_ansi.py
  python3 kellu_ansi.py --no-color
  python3 kellu_ansi.py --plain-frame

This is a visual prototype, not production terminal-capability detection.
"""
import argparse
import os
import sys

# KELLU accents: muted institutional blue, gold, copper, and light text.
BLUE = "\033[38;2;65;101;140m"
GOLD = "\033[38;2;200;166;106m"
COPPER = "\033[38;2;184;115;85m"
BOLD = "\033[1m"
DIM = "\033[2m"
RESET = "\033[0m"

def main():
    parser = argparse.ArgumentParser(description="Preview the KELLU terminal header.")
    parser.add_argument("--no-color", action="store_true",
                        help="disable ANSI color (bold remains enabled)")
    parser.add_argument("--no-bold", action="store_true",
                        help="disable bold styling")
    parser.add_argument("--plain-frame", action="store_true",
                        help="draw the frame without blue coloring")
    args = parser.parse_args()

    # Respect NO_COLOR. Avoid escape codes when output is piped or redirected.
    use_color = (
        sys.stdout.isatty()
        and "NO_COLOR" not in os.environ
        and not args.no_color
    )
    use_bold = sys.stdout.isatty() and not args.no_bold

    def style(code, enabled=True):
        return code if enabled else ""

    blue = style(BLUE, use_color and not args.plain_frame)
    gold = style(GOLD, use_color)
    copper = style(COPPER, use_color)
    bold = style(BOLD, use_bold)
    dim = style(DIM, use_color)
    reset = RESET if use_color or use_bold else ""

    # Keep frame width stable. ASCII-only borders also work in narrow fonts.
    width = 48
    inner = width - 2
    lines = [
        "    ◇",
        "‹ ◈ ›  " + "kellu",
        "    ◇     Secure and Modular AI Assistant",
    ]

    # The descriptor line is longer than the symbol lines, so size the frame to it.
    content_width = max(43, max(len(line) for line in lines))
    frame_width = content_width + 4
    top = "+" + "-" * (frame_width - 2) + "+"
    bottom = top

    print()
    print(f"{blue}{top}{reset}")
    for i, line in enumerate(lines):
        if i == 1:
            # Emphasize the name, while leaving the symbol in gold.
            prefix = "‹ "
            suffix = " ›  "
            print(f"{blue}|{reset}  {gold}{prefix}◈{suffix}{reset}{bold}kellu{reset}{' ' * max(1, frame_width - len(line) - 3)}{blue}|{reset}")
        elif i == 0:
            print(f"{blue}|{reset}  {gold}  ◇{reset}{' ' * (frame_width - 6)}{blue}|{reset}")
        elif i == 2:
            print(f"{blue}|{reset}  {gold}  ◇{reset}{copper}    Secure and Modular AI Assistant{reset}{' ' * (frame_width - len(line))}{blue}|{reset}")
        else:
            print(f"{blue}|{reset}  {dim}{line.strip()}{reset}{' ' * max(1, frame_width - len(line.strip()) - 3)}{blue}|{reset}")
    print(f"{blue}{bottom}{reset}")
    print()
    print(f"{copper}Secure and Modular AI Assistant{reset}")
    print(f"{dim}Multi-provider  |  Modular  |  Terminal-native{reset}")
    print(f"{gold}Environment preview:{reset} Unicode / color / bold")
    print()

if __name__ == "__main__":
    main()
