#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
prefix=${PREFIX:-"$HOME/.local"}
install -Dm755 bin/tetherust "$prefix/bin/tetherust"
if [[ -f lib/tetherust/tetherust-netguard ]]; then
    install -Dm755 lib/tetherust/tetherust-netguard "$prefix/lib/tetherust/tetherust-netguard"
    if [[ $EUID != 0 ]]; then printf 'Kill Switch requires a root-owned helper. See README for the explicit system installation command.
'; fi
fi
if [[ -f lib/tetherust/adb ]]; then install -Dm755 lib/tetherust/adb "$prefix/lib/tetherust/adb"; fi
if [[ -f lib/tetherust/ADB-NOTICE.txt ]]; then install -Dm644 lib/tetherust/ADB-NOTICE.txt "$prefix/lib/tetherust/ADB-NOTICE.txt"; fi
if [[ -f share/tetherust/Tetherust-Companion-0.3.0.apk ]]; then install -Dm644 share/tetherust/Tetherust-Companion-0.3.0.apk "$prefix/share/tetherust/Tetherust-Companion-0.3.0.apk"; fi
install -Dm644 tetherust.svg "$prefix/share/icons/hicolor/scalable/apps/tetherust.svg"
mkdir -p "$prefix/share/applications"
# Quote according to Desktop Entry Exec rules, not shell rules.
quoted=${prefix//\\/\\\\}; quoted=${quoted//\"/\\\"}; quoted=${quoted//\$/\\\$}; quoted=${quoted//\`/\\\`}; quoted=${quoted//%/%%}
while IFS= read -r line; do
    if [[ "$line" == Exec=* ]]; then printf 'Exec="%s/bin/tetherust" gui\n' "$quoted"; else printf '%s\n' "$line"; fi
done < tetherust.desktop > "$prefix/share/applications/tetherust.desktop"
printf 'Installed in %s. NetworkManager and desktop runtime libraries are required.\n' "$prefix"
