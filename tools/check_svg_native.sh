#!/bin/bash
# Strict actual Mac Catalyst/UIKit + existing pinned OpenKuiklyIOSRender archive probe.
# No dependency install, full application packaging, or application launch.
set -euo pipefail
REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$REPO/client/apps/macosApp"
POD="$ROOT/Pods/OpenKuiklyIOSRender/core-render-ios"
OUT="$REPO/client/build/ui-validation"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
if [[ -n "${KUIKLY_RENDER_ARCHIVE:-}" ]]; then
    ARCHIVE="$KUIKLY_RENDER_ARCHIVE"
else
    shopt -s nullglob
    ARCHIVES=("$HOME"/Library/Developer/Xcode/DerivedData/macosApp-*/Build/Products/Debug-maccatalyst/OpenKuiklyIOSRender/libOpenKuiklyIOSRender.a)
    if [[ ${#ARCHIVES[@]} -ne 1 ]]; then
        printf '%s\n' 'Set KUIKLY_RENDER_ARCHIVE to an existing pinned Mac Catalyst libOpenKuiklyIOSRender.a (no build/install performed).' >&2
        exit 2
    fi
    ARCHIVE="${ARCHIVES[0]}"
fi
if [[ ! -r "$ARCHIVE" || ! -r "$POD/Protocol/KuiklyRenderViewExportProtocol.h" || ! -d "$SDK/System/iOSSupport/System/Library/Frameworks/UIKit.framework" ]]; then
    printf '%s\n' 'Existing pinned renderer archive/Pods headers and Mac Catalyst UIKit SDK are required.' >&2
    exit 2
fi
mkdir -p "$OUT"
INCLUDES=(-I"$ROOT" -I"$POD/Protocol" -I"$POD/Extension" -I"$POD/Extension/Category" -I"$POD/Extension/BridgeProtocol" -I"$POD/Core" -I"$POD/View" -I"$POD/MacSupport" -I"$POD/Handler" -I"$POD/Handler/KuiklyTurboDisplay" -I"$POD/TDFCommon" -I"$POD/Extension/Modules" -I"$POD/Extension/Vendor" -I"$POD/Extension/AdvancedComps" -I"$POD/Extension/Components" -I"$POD/Extension/Components/Base" -I"$POD")
COMMON=(-target arm64-apple-ios15.0-macabi -isysroot "$SDK" -iframework "$SDK/System/iOSSupport/System/Library/Frameworks" -isystem "$SDK/System/iOSSupport/usr/include" -fobjc-arc -fmodules -Wall -Wextra -Werror -Wno-unused-parameter)
xcrun clang "${COMMON[@]}" "${INCLUDES[@]}" -fsyntax-only "$ROOT/AppDelegate.m" "$ROOT/XzSvgView.m" "$ROOT/XzSvgDocument.m" "$ROOT/XzSvgPath.m"
xcrun clang "${COMMON[@]}" "${INCLUDES[@]}" "$REPO/tools/svg_native_probe.m" "$ROOT/XzSvgView.m" "$ROOT/XzSvgDocument.m" "$ROOT/XzSvgPath.m" -ObjC "$ARCHIVE" -lc++ -framework UIKit -framework Foundation -framework QuartzCore -framework CoreGraphics -framework CoreText -framework AVFoundation -framework JavaScriptCore -o "$OUT/svg-native-probe"
# Only a freshly allocated, verified child temp directory is cleaned; old proof artifacts remain.
python3 - "$REPO" <<'PY'
import subprocess
import sys
import tempfile
from pathlib import Path
repo = Path(sys.argv[1]).resolve()
out = (repo / 'client/build/ui-validation').resolve()
with tempfile.TemporaryDirectory(prefix='svg-native-fixtures-', dir=out) as temp:
    fixture = Path(temp).resolve()
    assert fixture.parent == out and fixture.name.startswith('svg-native-fixtures-')
    icons = fixture / 'common/icons'
    icons.mkdir(parents=True)
    (icons / 'new-vector.svg').write_text("<svg viewBox='0 0 24 24' color='red'><path fill='none' stroke='currentColor' d='M1 1L23 23'/></svg>")
    (icons / 'oversize.svg').write_bytes(b' '*65537)
    (icons / 'malformed.svg').write_text("<svg viewBox='0 0 24 24'><path d='M0 0L'/></svg>")
    (icons / 'escape.svg').symlink_to('/etc/passwd')
    subprocess.run([str(out / 'svg-native-probe'), str(repo / 'client/shared/src/commonMain/assets'), str(fixture)], check=True)
PY
