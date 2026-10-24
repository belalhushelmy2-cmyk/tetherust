#!/usr/bin/env bash
set -euo pipefail
prefix=${PREFIX:-"$HOME/.local"}
rm -f "$prefix/bin/tetherust" "$prefix/share/applications/tetherust.desktop" "$prefix/share/icons/hicolor/scalable/apps/tetherust.svg"
rm -f "$prefix/lib/tetherust/tetherust-netguard" "$prefix/lib/tetherust/adb" "$prefix/lib/tetherust/ADB-NOTICE.txt" "$prefix/share/tetherust/Tetherust-Companion-0.3.0.apk"
printf 'Removed application files from %s. Disable Startup in Settings before removing; personal data remains.\n' "$prefix"
