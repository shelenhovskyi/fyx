# Run by the "Python with a welcome" terminal profile of fyx_demo_terminal_profiles, as
# `python -i ${package}/scripts/welcome.py`: Fyx puts the extension's installed folder in place of
# `${package}`. The `-i` keeps Python open afterwards.
import platform

print(f"Python {platform.python_version()}, started by a Fyx terminal profile.")
print("This script ships in the extension's package; the extension itself cannot read")
print("or type into this terminal. Try greet('you').")


def greet(name):
    return f"Hello, {name}, from the extension's package."
