#!/usr/bin/env python3
"""KELLU terminal header mockup using plain ASCII only.

Run:
  python3 kellu_ascii.py

No ANSI escape sequences, no Unicode characters, no color assumptions.
"""
def main():
    width = 48
    top = "+" + "-" * (width - 2) + "+"
    print()
    print(top)
    print("|   .                                           |")
    print("| < . >  KELLU                                  |")
    print("|   .    Secure and Modular AI Assistant        |")
    print(top)
    print()
    print("Secure and Modular AI Assistant")
    print("Multi-provider  |  Modular  |  Terminal-native")
    print("Environment preview: ASCII / monochrome")
    print()

if __name__ == "__main__":
    main()
