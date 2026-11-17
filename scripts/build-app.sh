#!/bin/zsh
# Builds Interleaved Study.app: the Rust engine as a static library, then the Swift app linked against it, then the
# bundle (Info.plist, icon, menu bar image), signed for this Mac.
#   scripts/build-app.sh            release build into build/Interleaved Study.app
#   scripts/build-app.sh debug      debug build (engine dev bridge available, see engine/src/devbridge.rs)
set -euo pipefail
cd "${0:A:h}/.."
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
# The engine is built for the macOS the app links against (app/Package.swift).
export MACOSX_DEPLOYMENT_TARGET=14.0

config=${1:-release}
[[ $config == release || $config == debug ]] || { echo "usage: $0 [release|debug]" >&2; exit 2 }

cargo build --manifest-path engine/Cargo.toml $([[ $config == release ]] && echo --release)
mkdir -p app/build/lib
cp "engine/target/$config/libinterleaved_engine.a" app/build/lib/

swift build --package-path app -c $config
bin="$(swift build --package-path app -c $config --show-bin-path)/InterleavedStudy"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' engine/Cargo.toml | head -1)
out="build/Interleaved Study.app"
rm -rf "$out"
mkdir -p "$out/Contents/MacOS" "$out/Contents/Resources"
cp "$bin" "$out/Contents/MacOS/Interleaved Study"
cp app/Resources/AppIcon.icns app/Resources/trayTemplate.png app/Resources/trayTemplate@2x.png "$out/Contents/Resources/"
# The accent colour (a warm grey: chrome holds no hue) for sidebar selection, focus and switches.
xcrun actool app/Resources/Assets.xcassets --compile "$out/Contents/Resources" --platform macosx --minimum-deployment-target 14.0 >/dev/null
sed "s/__VERSION__/$version/g" app/Resources/Info.plist > "$out/Contents/Info.plist"
codesign --force --sign - "$out"
echo "Built $out"
