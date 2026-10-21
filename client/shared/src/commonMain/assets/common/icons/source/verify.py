#!/usr/bin/env python3
"""Verify original SVG-only admin resources using Python's standard library.

Read-only by default (also accepts --check). --write-manifest updates metadata only;
no raster conversion, image framework, network or third-party dependency is used.
Optional --generated-registry PATH decodes actual generated Kotlin string literals
and checks them against every canonical common SVG. --web-dir PATH verifies no
managed SVG copies remain. --bundle-zip PATH checks native/shared asset packaging.
Copied SVG publication (--published-dir) is obsolete and rejected explicitly.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile

BASE = Path(__file__).resolve().parent.parent
ROOT = next(p for p in BASE.parents if (p / "client/shared/build.gradle.kts").is_file())
KOTLIN = ROOT / "client/shared/src/commonMain/kotlin/com/xiaozhi/admin"
RENDERER = {"common": "XzSvgView", "web": "inline_svg", "native": "CAShapeLayer", "bitmapFallback": False}
PROVENANCE = "Original Xiaozhi project vectors, not copied from a third-party icon set"


def require(value, message):
    if not value:
        raise ValueError(message)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def entries():
    enum = re.search(r"enum class IconToken\s*\{([^}]+)}", (KOTLIN / "UiContracts.kt").read_text())
    require(enum is not None, "IconToken enum missing")
    tokens = re.findall(r"\b[A-Z][A-Za-z]+\b", enum.group(1))
    source = (KOTLIN / "AdminIcons.kt").read_text()
    pairs = re.findall(r'IconToken\.(\w+) -> "([a-z-]+)"', source)
    mapping = dict(pairs)
    require(bool(tokens) and len(tokens) == len(set(tokens)) == len(pairs) == len(mapping), "Expected nonempty unique icon tokens/mappings")
    require(set(tokens) == set(mapping), "IconToken mapping incomplete")
    require(len(set(mapping.values())) == len(tokens), "Icon asset names must be unique")
    require('"assets://common/icons/source/${assetName()}.svg"' in source, "Public URI must remain canonical SVG")
    require({p.stem for p in (BASE / "source").glob("*.svg")} == set(mapping.values()), "SVG file catalogue differs")
    native = (ROOT / "client/apps/macosApp/XzSvgView.m").read_text()
    require("@implementation XzSvgView" in native and "XzSvgDocument" in native, "Dedicated native SVG vector view missing")
    require("assets://common/" in native and "URLByResolvingSymlinksInPath" in native, "Native bundled-common SVG root policy missing")
    require("UIImage" not in native and ".png" not in native.lower(), "Native SVG view must not use bitmap fallback")
    app = (ROOT / "client/apps/macosApp/AppDelegate.m").read_text()
    require("AdminAssetAdapter" not in app, "Obsolete bitmap adapter still registered")
    for ext in ("h", "m"):
        require(not (ROOT / f"client/apps/macosApp/AdminAssetAdapter.{ext}").exists(), "Obsolete bitmap adapter source remains")
    return [(token, mapping[token]) for token in tokens]


def verify_vectors(icons):
    require(not any(p.is_file() and p.suffix.lower() == ".png" for p in BASE.rglob("*")), "PNG resources are forbidden in this SVG-only catalogue")
    require(not (BASE / "source/render.py").exists(), "Obsolete bitmap generator remains")
    license_text = (BASE / "LICENSE.txt").read_text()
    require("SPDX-License-Identifier: CC0-1.0" in license_text, "CC0 dedication missing")
    for _, name in icons:
        path = BASE / "source" / (name + ".svg")
        data = path.read_bytes()
        require(len(data) <= 65536, f"SVG exceeds native byte budget: {name}")
        text = data.decode("utf-8")
        require("<!DOCTYPE" not in text and "<!ENTITY" not in text and "url(" not in text.lower(), f"Untrusted SVG references: {name}")
        svg = ET.fromstring(data)
        require(svg.tag == "{http://www.w3.org/2000/svg}svg" and svg.get("viewBox") == "0 0 24 24", f"SVG viewBox/namespace mismatch: {name}")
        require("width" not in svg.attrib and "height" not in svg.attrib, f"SVG root dimensions must be container-driven: {name}")
        painted = False
        for node in svg.iter():
            require(node.tag.rsplit("}", 1)[-1] in {"svg", "g", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon"}, f"Unsupported SVG element: {name}")
            for attr, value in node.attrib.items():
                require(not attr.lower().startswith("on") and "href" not in attr.lower() and attr != "style", f"External/active SVG attribute: {name}")
                if attr in {"fill", "stroke"}:
                    require(value in {"none", "currentColor"}, f"Paint must use currentColor or none: {name}")
                    painted |= value == "currentColor"
        require(painted, f"SVG has no currentColor paint: {name}")


def manifest_for(icons):
    return {"license": "CC0-1.0", "provenance": PROVENANCE, "format": "svg", "viewBox": "0 0 24 24",
            "color": "currentColor", "renderer": RENDERER, "licenseSha256": sha256(BASE / "LICENSE.txt"),
            "icons": [{"token": token, "name": name, "svgSha256": sha256(BASE / "source" / (name + ".svg"))} for token, name in icons]}


def common_sources():
    root = ROOT / "client/shared/src/commonMain/assets/common"
    require(root.resolve() == root, "Common SVG source root is aliased")
    result, total = {}, 0
    for path in sorted(root.rglob("*.svg")):
        relative = path.relative_to(root).as_posix()
        require(re.fullmatch(r"(?:[A-Za-z0-9_-]+/)*[A-Za-z0-9_-]+\.svg", relative), f"Unsafe common SVG path: {relative}")
        require(path.resolve() == path, f"Aliased common SVG path: {relative}")
        data = path.read_bytes()
        total += len(data)
        require(len(data) <= 65536 and total <= 2 * 1024 * 1024, "Common SVG byte budget exceeded")
        result["assets://common/" + relative] = data.decode("utf-8")
    require(result, "Common SVG catalogue is empty")
    return result


def decode_kotlin_literal(literal):
    # Kotlin's only non-JSON string escape emitted by the generator is literal \$.
    index = 1
    while index < len(literal) - 1:
        if literal[index] == "\\":
            index += 2
        else:
            require(literal[index] != "$", "Unescaped Kotlin interpolation in generated SVG literal")
            index += 1
    return json.loads(literal.replace("\\$", "$"))


def verify_registry(path):
    text = path.read_text(encoding="utf-8")
    require("package com.xiaozhi.admin" in text and "internal object BundledSvgSources" in text and "val bySource: Map<String, String> = mapOf(" in text, "Generated registry API differs")
    literal = r'"(?:[^"\\]|\\.)*"'
    pairs = re.findall(r"^\s*(" + literal + r") to (" + literal + r"),\s*$", text, re.M)
    decoded = [(decode_kotlin_literal(key), decode_kotlin_literal(value)) for key, value in pairs]
    actual = dict(decoded)
    expected = common_sources()
    require(len(decoded) == len(actual) == len(expected) and actual == expected, "Generated registry missing, duplicate or stale SVG bytes")


def verify_web_dir(directory, icons):
    require(directory.is_dir(), "Published Web directory missing")
    root = directory / "assets/common/icons"
    paths = [root / relative for relative in ("manifest.json", "LICENSE.txt", "source/render.py")]
    paths += [root / (name + ".png") for _, name in icons]
    paths += [directory / "assets/common" / source.removeprefix("assets://common/") for source in common_sources()]
    require(not any(path.exists() or path.is_symlink() for path in paths), "Duplicate/obsolete copied Web SVG or metadata remains; Web must use the embedded registry")


def verify_zip(path, icons):
    with zipfile.ZipFile(path) as bundle:
        names = bundle.namelist()
        require(not any("common/icons/" in n and n.lower().endswith(".png") for n in names), "Bundle contains obsolete PNG icons")
        for relative in ["manifest.json", "LICENSE.txt"] + [f"source/{name}.svg" for _, name in icons]:
            matches = [n for n in names if n.endswith("common/icons/" + relative)]
            require(len(matches) == 1, f"Bundle missing/duplicate resource: {relative}")
            require(bundle.read(matches[0]) == (BASE / relative).read_bytes(), f"Bundle resource stale: {relative}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Read-only verification (the default)")
    parser.add_argument("--write-manifest", action="store_true", help="Update SVG metadata only, never source artwork")
    parser.add_argument("--generated-registry", type=Path)
    parser.add_argument("--web-dir", type=Path)
    parser.add_argument("--published-dir", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--bundle-zip", type=Path)
    args = parser.parse_args()
    require(not (args.check and args.write_manifest), "Choose --check OR --write-manifest")
    icons = entries()
    verify_vectors(icons)
    expected = manifest_for(icons)
    manifest = BASE / "manifest.json"
    if args.write_manifest:
        manifest.write_text(json.dumps(expected, indent=2) + "\n")
    require(json.loads(manifest.read_text()) == expected, "SVG manifest metadata or hashes differ")
    require(args.published_dir is None, "Copied SVG publication is obsolete; use --generated-registry and --web-dir")
    if args.generated_registry:
        verify_registry(args.generated_registry)
    if args.web_dir:
        verify_web_dir(args.web_dir, icons)
    if args.bundle_zip:
        verify_zip(args.bundle_zip, icons)
    print(f"Verified {len(icons)} SVG-only IconTokens: currentColor, canonical URI, native vector/root policy, CC0 and SHA-256")


if __name__ == "__main__":
    main()
