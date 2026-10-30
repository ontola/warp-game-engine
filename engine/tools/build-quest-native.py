#!/usr/bin/env python3
"""Build/sign Growing Guns' native ARM64 APK; no Gradle or browser frontend.
Requires Rust aarch64-linux-android, cargo-ndk, Android SDK/NDK and Java 17+.
The debug signing key stays outside Git. --install is an explicit adb install.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2]
LOADER_VERSION = "1.1.65"
LOADER_SHA256 = "a298c326085b09181c85235fb04d37510939bd30938ed4322c093c09780e8643"


def run(*args, **kwargs):
    subprocess.run([str(x) for x in args], cwd=ROOT, check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skip-build", action="store_true", help="Package a previously built native library")
    parser.add_argument("--install", action="store_true", help="Install on the single authorized adb device")
    parser.add_argument("--audio", type=Path, help="Optional original Growing Guns recordings directory")
    args = parser.parse_args()
    sdk = Path(os.environ.get("ANDROID_HOME", os.environ.get("ANDROID_SDK_ROOT", str(Path.home()/"Library/Android/sdk"))))
    build_tools = sdk/"build-tools/36.0.0"
    android_jar = sdk/"platforms/android-35/android.jar"
    if not (build_tools/"aapt2").is_file() or not android_jar.is_file():
        raise SystemExit("Install Android SDK platform 35 and build-tools 36.0.0, or set ANDROID_HOME.")
    ndk = Path(os.environ.get("ANDROID_NDK_HOME", str(sdk/"ndk/28.2.13676358")))
    env = dict(os.environ, ANDROID_HOME=str(sdk), ANDROID_NDK_HOME=str(ndk), CARGO_BUILD_BUILD_DIR=str(ROOT/"build/cargo-cache"))
    if "JAVA_HOME" not in env and Path("/usr/libexec/java_home").exists():
        env["JAVA_HOME"] = subprocess.check_output(["/usr/libexec/java_home", "-v", "17"], text=True).strip()
    java = Path(env["JAVA_HOME"])/"bin" if "JAVA_HOME" in env else None
    out = ROOT/"build/quest-native"
    out.mkdir(parents=True, exist_ok=True)
    if not args.skip_build:
        run("cargo", "ndk", "-t", "arm64-v8a", "-P", "29", "build", "--locked", "-p", "growing-guns-app", "--lib", "--profile", "quest", env=env)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT, text=True))
    library = Path(metadata["target_directory"])/"aarch64-linux-android/quest/libgrowing_guns_app.so"
    if not library.is_file():
        raise SystemExit(f"Missing native library: {library}")
    loader = out/f"openxr-loader-{LOADER_VERSION}.aar"
    if not loader.is_file():
        urllib.request.urlretrieve(f"https://repo.maven.apache.org/maven2/org/khronos/openxr/openxr_loader_for_android/{LOADER_VERSION}/openxr_loader_for_android-{LOADER_VERSION}.aar", loader)
    if hashlib.sha256(loader.read_bytes()).hexdigest() != LOADER_SHA256:
        raise SystemExit("OpenXR loader checksum mismatch")
    assets = out/"assets"
    assets.mkdir(exist_ok=True)
    with zipfile.ZipFile(assets/"growing-guns.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        game = ROOT/"games/growing-guns"
        for folder in ["assets", "mods", "sounds"]:
            for file in sorted((game/folder).rglob("*")):
                if file.is_file():
                    archive.write(file, file.relative_to(game))
        if args.audio:
            for file in sorted(args.audio.rglob("*")):
                if file.is_file():
                    archive.write(file, Path("audio")/file.relative_to(args.audio))
    unsigned = out/"growing-guns-unsigned.apk"
    run(build_tools/"aapt2", "link", "-I", android_jar, "--manifest", ROOT/"games/growing-guns/android/AndroidManifest.xml", "-A", assets, "-o", unsigned)
    host = "darwin-x86_64" if os.uname().sysname == "Darwin" else "linux-x86_64"
    cxx = ndk/f"toolchains/llvm/prebuilt/{host}/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so"
    with zipfile.ZipFile(unsigned, "a", zipfile.ZIP_STORED) as apk:
        apk.write(library, "lib/arm64-v8a/libgrowing_guns_app.so")
        apk.write(cxx, "lib/arm64-v8a/libc++_shared.so")
        with zipfile.ZipFile(loader) as aar:
            apk.writestr("lib/arm64-v8a/libopenxr_loader.so", aar.read("jni/arm64-v8a/libopenxr_loader.so"))
            # Keep the upstream license alongside the bundled loader.
            for name in aar.namelist():
                if name.startswith("META-INF/") and ("LICENSE" in name.upper() or "NOTICE" in name.upper()):
                    apk.writestr("assets/openxr/"+Path(name).name, aar.read(name))
    aligned = out/"growing-guns-aligned.apk"
    run(build_tools/"zipalign", "-P", "16", "-f", "4", unsigned, aligned)
    key = Path.home()/".android/warp-quest-debug.keystore"
    key.parent.mkdir(exist_ok=True)
    if not key.exists():
        run(java/"keytool" if java else "keytool", "-genkeypair", "-keystore", key, "-storepass", "android", "-keypass", "android", "-alias", "androiddebugkey", "-keyalg", "RSA", "-keysize", "2048", "-validity", "10000", "-dname", "CN=Warp Quest Debug", env=env)
    apk = out/"growing-guns-quest2.apk"
    run(build_tools/"apksigner", "sign", "--ks", key, "--ks-key-alias", "androiddebugkey", "--ks-pass", "pass:android", "--key-pass", "pass:android", "--out", apk, aligned, env=env)
    run(build_tools/"apksigner", "verify", "--verbose", apk, env=env)
    run(build_tools/"zipalign", "-c", "-P", "16", "4", apk)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    (out/"build.json").write_text(json.dumps({"revision":revision,"dirty":bool(subprocess.check_output(["git","status","--porcelain"],cwd=ROOT)),
        "apk":str(apk),"sha256":hashlib.sha256(apk.read_bytes()).hexdigest(),"openxr_loader":LOADER_VERSION,
        "target":"aarch64-linux-android","profile":"quest","headset_tested":False}, indent=2)+"\n")
    print(f"Native APK: {apk} ({apk.stat().st_size/1024/1024:.1f} MiB). Hardware validation still required.")
    if args.install:
        run(sdk/"platform-tools/adb", "install", "-r", apk)


if __name__ == "__main__":
    main()
