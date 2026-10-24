#!/usr/bin/env bash
# Pure Android SDK/Kotlin build. Does not discover, contact or install on any device.
set -euo pipefail
cd "$(dirname "$0")"
: "${ANDROID_HOME:?Set ANDROID_HOME to an installed Android SDK}"
: "${KOTLIN_COMPILER_HOME:?Set KOTLIN_COMPILER_HOME to a Kotlin distribution or Gradle lib directory}"
build_tools="$ANDROID_HOME/build-tools/35.0.0"
android="$ANDROID_HOME/platforms/android-35/android.jar"
rm -rf build/classes build/dex
mkdir -p build/classes build/dex ../dist
compiler="$KOTLIN_COMPILER_HOME"
stdlib=$(find "$compiler" -maxdepth 1 -name 'kotlin-stdlib-[0-9]*.jar' | sort | head -1)
annotations=$(find "$compiler" -maxdepth 1 -name 'annotations-*.jar' | sort | head -1)
[[ -f "$stdlib" && -f "$android" ]]
java -cp "$compiler/*" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 1.8 \
    -classpath "$android:$stdlib:$annotations" -d build/classes src/org/tetherust/companion/*.kt
jar cf build/classes.jar -C build/classes .
"$build_tools/d8" --release --min-api 24 --lib "$android" --output build/dex build/classes.jar "$stdlib"
"$build_tools/aapt2" link -A assets -I "$android" --manifest AndroidManifest.xml --version-code 3 --version-name 0.3.0 -o build/unsigned.apk
(cd build/dex; zip -q ../unsigned.apk classes*.dex)
"$build_tools/zipalign" -f -p 4 build/unsigned.apk build/aligned.apk
# Stable private signing identity for user-installable updates. Never put this key/password in Git.
keydir="${TETHERUST_ANDROID_SIGNING_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/tetherust/android-signing}"
mkdir -p "$keydir"; chmod 700 "$keydir"
if [[ ! -f "$keydir/password" ]]; then (umask 077; openssl rand -hex 24 > "$keydir/password"); fi
export TETHERUST_STOREPASS; TETHERUST_STOREPASS=$(cat "$keydir/password")
if [[ ! -f "$keydir/release.jks" ]]; then
    (umask 077; keytool -genkeypair -keystore "$keydir/release.jks" -alias tetherust -keyalg RSA -keysize 3072 -validity 10000 \
        -storepass:env TETHERUST_STOREPASS -keypass:env TETHERUST_STOREPASS -dname 'CN=Tetherust Companion, O=Tetherust' -noprompt)
fi
"$build_tools/apksigner" sign --ks "$keydir/release.jks" --ks-key-alias tetherust --ks-pass env:TETHERUST_STOREPASS --key-pass env:TETHERUST_STOREPASS \
    --out ../dist/Tetherust-Companion-0.3.0.apk build/aligned.apk
"$build_tools/apksigner" verify --verbose ../dist/Tetherust-Companion-0.3.0.apk
sha256sum ../dist/Tetherust-Companion-0.3.0.apk > ../dist/Tetherust-Companion-0.3.0.apk.sha256
