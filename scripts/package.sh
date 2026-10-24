#!/usr/bin/env bash
# Development artifacts: build on an older Linux baseline before broad redistribution.
set -euo pipefail
cd "$(dirname "$0")/.."
version=0.3.0
if [[ -z ${TETHERUST_BINARY:-} ]]; then
    cargo build --release --locked
    target=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
    TETHERUST_BINARY="$target/release/tetherust"
fi
binary=$(realpath "$TETHERUST_BINARY")
output=$(realpath -m "${TETHERUST_OUTPUT:-dist}")
mkdir -p "$output"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
bundle="$work/tetherust-$version"
mkdir -p "$bundle/bin" "$bundle/share/tetherust" "$bundle/lib/tetherust"
install -m755 "$binary" "$bundle/bin/tetherust"
helper="$(dirname "$binary")/tetherust-netguard"
[[ -f "$helper" ]] || { echo 'Build tetherust-netguard alongside the application first' >&2; exit 1; }
install -m755 "$helper" "$bundle/lib/tetherust/tetherust-netguard"
if [[ -f dist/Tetherust-Companion-0.3.0.apk ]]; then cp dist/Tetherust-Companion-0.3.0.apk "$bundle/share/tetherust/"; fi
if [[ -n ${TETHERUST_ADB:-} ]]; then
    install -m755 "$TETHERUST_ADB" "$bundle/lib/tetherust/adb"
    if [[ -f "$(dirname "$TETHERUST_ADB")/NOTICE.txt" ]]; then cp "$(dirname "$TETHERUST_ADB")/NOTICE.txt" "$bundle/lib/tetherust/ADB-NOTICE.txt"; fi
fi
cp packaging/tetherust.desktop packaging/tetherust.svg packaging/install.sh packaging/uninstall.sh README.md LICENSE ARCHITECTURE.md "$bundle/"
chmod 755 "$bundle/install.sh" "$bundle/uninstall.sh"
printf 'Development build produced on: %s\nMinimum glibc symbol baseline: %s\nHardware acceptance: NOT PERFORMED\n' "$(uname -sr)" "$(objdump -T "$binary" | sed -n 's/.*(GLIBC_\([^)]*\)).*/\1/p' | sort -V | tail -1)" > "$bundle/BUILD_INFO.txt"
if [[ -f "$output/THIRD-PARTY-NOTICES.txt" ]]; then cp "$output/THIRD-PARTY-NOTICES.txt" "$bundle/"; fi
tar -C "$work" -czf "$output/tetherust-$version-linux-x86_64.tar.gz" "tetherust-$version"
# Source archive accompanies GPL binaries and includes all scripts/integration sources.
mkdir -p "$work/source/tetherust-$version"
tar --exclude='./target' --exclude='./dist' --exclude='./android/build' --exclude='./.git' --exclude='./packaging/arch/pkg' --exclude='./packaging/arch/src' --exclude='*.pkg.tar.*' -cf - . | tar -C "$work/source/tetherust-$version" -xf -
tar -C "$work/source" -czf "$output/tetherust-$version-source.tar.gz" "tetherust-$version"
cp packaging/arch/PKGBUILD "$output/PKGBUILD"
source_sha=$(sha256sum "$output/tetherust-$version-source.tar.gz" | cut -d' ' -f1)
sed -i "s/sha256sums=('SKIP')/sha256sums=('$source_sha')/" "$output/PKGBUILD"
# Build a valid Debian archive using standard ar/tar; no dpkg-dev or root installation needed.
deb="$work/deb"; mkdir -p "$deb/control" "$deb/data/usr/bin" "$deb/data/usr/share/applications" "$deb/data/usr/share/icons/hicolor/scalable/apps" "$deb/data/usr/share/doc/tetherust"
install -m755 "$binary" "$deb/data/usr/bin/tetherust"
cp packaging/tetherust.desktop "$deb/data/usr/share/applications/"
cp packaging/tetherust.svg "$deb/data/usr/share/icons/hicolor/scalable/apps/"
cp README.md LICENSE "$bundle/BUILD_INFO.txt" "$deb/data/usr/share/doc/tetherust/"
mkdir -p "$deb/data/usr/share/tetherust" "$deb/data/usr/lib/tetherust"
cp -a "$bundle/share/tetherust/." "$deb/data/usr/share/tetherust/"
cp -a "$bundle/lib/tetherust/." "$deb/data/usr/lib/tetherust/"
if [[ -f "$output/THIRD-PARTY-NOTICES.txt" ]]; then cp "$output/THIRD-PARTY-NOTICES.txt" "$deb/data/usr/share/doc/tetherust/"; fi
cat > "$deb/control/control" <<CONTROL
Package: tetherust
Version: $version-1
Architecture: amd64
Maintainer: Tetherust contributors <noreply@example.invalid>
Section: net
Priority: optional
Depends: libc6, network-manager, policykit-1, nftables, iproute2, libfontconfig1, libxkbcommon0, libwayland-client0, libx11-6, libxcursor1, libxi6, libxrandr2, fonts-dejavu-core
Recommends: wl-clipboard | xclip
Description: Native Android tethering manager (development build)
 Native Rust and Slint USB tethering with NetworkManager integration.
 Hardware acceptance and cross-distribution compatibility remain unverified.
CONTROL
baseline=$(objdump -T "$binary" | sed -n 's/.*(GLIBC_\([^)]*\)).*/\1/p' | sort -V | tail -1)
sed -i "s/Depends: libc6,/Depends: libc6 (>= $baseline),/" "$deb/control/control"
printf '2.0\n' > "$deb/debian-binary"
tar --owner=0 --group=0 -C "$deb/control" -czf "$deb/control.tar.gz" .
tar --owner=0 --group=0 -C "$deb/data" -czf "$deb/data.tar.gz" .
(cd "$deb"; ar cr "$output/tetherust_${version}-1_amd64.deb" debian-binary control.tar.gz data.tar.gz)
if [[ -n ${APPIMAGETOOL:-} && -n ${APPIMAGE_RUNTIME:-} ]]; then
    appdir="$work/Tetherust.AppDir"
    mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" "$appdir/usr/share/fonts"
    cp "$binary" "$appdir/usr/bin/tetherust"
    mkdir -p "$appdir/usr/share/tetherust" "$appdir/usr/lib/tetherust"
    cp -a "$bundle/share/tetherust/." "$appdir/usr/share/tetherust/"
    cp -a "$bundle/lib/tetherust/." "$appdir/usr/lib/tetherust/"
    cp packaging/tetherust.desktop packaging/tetherust.svg "$appdir/"
    cp LICENSE "$appdir/"
    if [[ -d /usr/share/licenses/common ]]; then cp -a /usr/share/licenses/common "$appdir/system-library-licenses"; fi
    if [[ -f "$output/THIRD-PARTY-NOTICES.txt" ]]; then cp "$output/THIRD-PARTY-NOTICES.txt" "$appdir/"; fi
    # Include direct dependencies plus libraries Winit/Fontconfig load at runtime.
    libraries=("$binary")
    for library in libwayland-client.so.0 libwayland-cursor.so.0 libwayland-egl.so.1 libfontconfig.so.1 libxkbcommon.so.0 libxkbcommon-x11.so.0 libX11.so.6 libX11-xcb.so.1 libXcursor.so.1 libXi.so.6 libXrandr.so.2; do
        location=$(ldconfig -p | awk -v name="$library" '$1 == name && /x86-64/ && !found {print $NF; found=1}')
        if [[ -n "$location" ]]; then libraries+=("$location"); cp -L "$location" "$appdir/usr/lib/"; fi
    done
    while read -r library; do
        case "$(basename "$library")" in libc.so.*|libm.so.*|libpthread.so.*|libdl.so.*|librt.so.*|ld-linux*|libresolv.so.*) continue ;; esac
        cp -L "$library" "$appdir/usr/lib/"
    done < <(ldd "${libraries[@]}" | awk '/=> \// {print $3}' | sort -u)
    font=$(fc-match -f '%{file}' 'DejaVu Sans')
    cp "$font" "$appdir/usr/share/fonts/DejaVuSans.ttf"
    # Fontconfig may substitute the user’s font. Include its actual package license.
    font_package=$(pacman -Qqo "$font" 2>/dev/null || true)
    if [[ -n "$font_package" && -d "/usr/share/licenses/$font_package" ]]; then
        cp -a "/usr/share/licenses/$font_package" "$appdir/usr/share/fonts/licenses"
    elif [[ "$font" == *dejavu* ]]; then
        cp /usr/share/licenses/ttf-dejavu/LICENSE "$appdir/usr/share/fonts/LICENSE"
    else
        echo "Could not locate the bundled font license: $font" >&2; exit 1
    fi
    cat > "$appdir/AppRun" <<'RUN'
#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")" && pwd)
export LD_LIBRARY_PATH="$root/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
# System Fontconfig remains authoritative; a DejaVu fallback file is included for packagers.
export FONTCONFIG_PATH="${FONTCONFIG_PATH:-/etc/fonts}"
exec "$root/usr/bin/tetherust" "$@"
RUN
    chmod 755 "$appdir/AppRun"
    ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" --no-appstream --runtime-file "$APPIMAGE_RUNTIME" "$appdir" "$output/Tetherust-$version-x86_64.AppImage"
fi
(cd "$output"; sha256sum "tetherust-$version-linux-x86_64.tar.gz" "tetherust-$version-source.tar.gz" "tetherust_${version}-1_amd64.deb" > SHA256SUMS; if [[ -f "Tetherust-Companion-$version.apk" ]]; then sha256sum "Tetherust-Companion-$version.apk" >> SHA256SUMS; fi; if [[ -f "Tetherust-$version-x86_64.AppImage" ]]; then sha256sum "Tetherust-$version-x86_64.AppImage" >> SHA256SUMS; fi)
printf 'Artifacts: %s\n' "$output"
