"""Check generated mappings against a supplied native Fontconfig DLL.

Run cargo test first to generate target/fontconfig-native fixtures, then:
    python tests/verify_fontconfig.py <path-to-libfontconfig-1.dll>
All fonts/configuration come from target; the game installation is read only.
"""

import ctypes as c
import os
from pathlib import Path
import sys
import xml.etree.ElementTree as ET


def main() -> None:
    library = Path(sys.argv[1]).resolve(strict=True)
    search = os.add_dll_directory(str(library.parent))
    fc = c.CDLL(str(library))
    pointer = c.c_void_p
    signatures = {
        "FcGetVersion": (c.c_int, []),
        "FcConfigCreate": (pointer, []),
        "FcConfigDestroy": (None, [pointer]),
        "FcConfigParseAndLoad": (c.c_int, [pointer, c.c_char_p, c.c_int]),
        "FcConfigAppFontAddFile": (c.c_int, [pointer, c.c_char_p]),
        "FcNameParse": (pointer, [c.c_char_p]),
        "FcConfigSubstitute": (c.c_int, [pointer, pointer, c.c_int]),
        "FcDefaultSubstitute": (None, [pointer]),
        "FcFontMatch": (pointer, [pointer, pointer, c.POINTER(c.c_int)]),
        "FcPatternGetString": (c.c_int, [pointer, c.c_char_p, c.c_int, c.POINTER(c.c_char_p)]),
        "FcPatternGetInteger": (c.c_int, [pointer, c.c_char_p, c.c_int, c.POINTER(c.c_int)]),
        "FcPatternGetDouble": (c.c_int, [pointer, c.c_char_p, c.c_int, c.POINTER(c.c_double)]),
        "FcPatternDestroy": (None, [pointer]),
    }
    for name, (result, arguments) in signatures.items():
        function = getattr(fc, name)
        function.restype = result
        function.argtypes = arguments

    root = Path(__file__).resolve().parents[1] / "target/fontconfig-native"
    count = 0
    requests = [
        ("Stratum2", False, 80),
        ("Stratum2:weight=80", False, 80),
        ("Stratum2:weight=180", True, 180),
        ("Stratum2:weight=200", True, 200),
        ("Stratum2 Bold", True, 200),
        ("Stratum2 Black", True, 210),
        ("Stratum2 Semibold", True, 180),
        ("Arial:style=Bold", True, 200),
        ("Arial:style=Bold Italic", True, 200),
        ("Arial:style=Semibold", True, 180),
        ("Stratum2 Bold:weight=210", True, 210),
    ]
    print(f"Native Fontconfig version: {fc.FcGetVersion()}")
    for scenario in ["same-family", "different-family", "single"]:
        folder = root / scenario
        for filename in ["fonts.conf", "42-repl-global.conf"]:
            assert ET.parse(folder / filename).getroot().tag == "fontconfig"
        config = fc.FcConfigCreate()
        assert config
        try:
            path = str(folder / "42-repl-global.conf").encode("utf-8")
            assert fc.FcConfigParseAndLoad(config, path, 1)
            faces = ["cs2fc-regular.ttf"]
            if scenario != "single":
                faces.append("cs2fc-bold.ttf")
            for filename in faces:
                assert fc.FcConfigAppFontAddFile(config, str(folder / filename).encode("utf-8"))
            for request, wants_bold, expected_weight in requests:
                pattern = fc.FcNameParse(f"{request}:pixelsize=20".encode("utf-8"))
                assert pattern
                match = None
                try:
                    assert fc.FcConfigSubstitute(config, pattern, 0)
                    weight = c.c_int()
                    assert fc.FcPatternGetInteger(pattern, b"weight", 0, c.byref(weight)) == 0
                    assert weight.value == expected_weight, (scenario, request, weight.value)
                    fc.FcDefaultSubstitute(pattern)
                    result = c.c_int()
                    match = fc.FcFontMatch(config, pattern, c.byref(result))
                    assert match and result.value == 0
                    filename = c.c_char_p()
                    assert fc.FcPatternGetString(match, b"file", 0, c.byref(filename)) == 0
                    expected = "cs2fc-bold.ttf" if wants_bold and scenario != "single" else "cs2fc-regular.ttf"
                    assert Path(filename.value.decode("utf-8")).name == expected, (scenario, request, filename.value)
                    pixels = c.c_double()
                    assert fc.FcPatternGetDouble(match, b"pixelsize", 0, c.byref(pixels)) == 0
                    assert abs(pixels.value - 22.0) < 0.001, (scenario, request, pixels.value)
                    print(f"{scenario}: {request} -> {expected}, {pixels.value:g}px")
                    count += 1
                finally:
                    if match:
                        fc.FcPatternDestroy(match)
                    fc.FcPatternDestroy(pattern)
        finally:
            fc.FcConfigDestroy(config)
    search.close()
    print(f"Passed {count} native matching checks; XML valid in all scenarios.")


if __name__ == "__main__":
    main()
