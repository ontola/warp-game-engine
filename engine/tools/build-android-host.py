#!/usr/bin/env python3
"""Build the native Splitscreen Club host for Android phones: the Game Room
and its games (the browser host, lobbies/game-room/examples/browser) in a
NativeActivity with Vulkan, as build/android-host/splitscreen-host.apk.
No Gradle: aapt2 packs the manifest and the games, the native library goes
in beside them, zipalign and apksigner finish it.

Needs Rust's aarch64-linux-android target, cargo-ndk, Android SDK platform 35
with build-tools 36.0.0, the NDK (ANDROID_HOME and ANDROID_NDK_HOME), Java 17+.
The debug signing key lives in ~/.android/splitscreen-host-debug.keystore
(or --keystore with $HOST_KEYSTORE_PASSWORD), never in the repository.

    build-android-host.py [--games neon-siege,stack-together] [--install]
    build-android-host.py --lib-only        # only the .so and the bundle,
                                            # for the Splitscreen Club app
See engine/docs/android-host.md.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'build/android-host'
CRATE, LIB = 'game-room-android', 'splitscreen_host'
MANIFEST = ROOT / 'lobbies/game-room/android/android/AndroidManifest.xml'


def run(*args, **kw):
    subprocess.run([str(a) for a in args], cwd=ROOT, check=True, **kw)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--games', help='Luau game folders on the shelf (default: the browser build\'s)')
    p.add_argument('--skip-build', action='store_true', help='package the library built before')
    p.add_argument('--skip-assets', action='store_true', help='keep the games and room light from the last run')
    p.add_argument('--lib-only', action='store_true', help='stop after the library and the bundle (build/android-host/lib, build/android-host/web)')
    p.add_argument('--version-code', type=int, default=int(os.environ.get('HOST_VERSION_CODE', '1')))
    p.add_argument('--keystore', type=Path, help='release keystore (password in $HOST_KEYSTORE_PASSWORD, alias $HOST_KEY_ALIAS)')
    p.add_argument('--install', action='store_true', help='adb install on the one attached phone')
    args = p.parse_args()
    sdk = Path(os.environ.get('ANDROID_HOME', os.environ.get('ANDROID_SDK_ROOT', str(Path.home() / 'Library/Android/sdk'))))
    ndk = Path(os.environ.get('ANDROID_NDK_HOME', str(sdk / 'ndk/28.2.13676358')))
    tools, jar = sdk / 'build-tools/36.0.0', sdk / 'platforms/android-35/android.jar'
    if not args.lib_only and (not (tools / 'aapt2').is_file() or not jar.is_file()):
        sys.exit('Install Android SDK platform 35 and build-tools 36.0.0, or set ANDROID_HOME.')
    env = dict(os.environ, ANDROID_HOME=str(sdk), ANDROID_NDK_HOME=str(ndk))
    OUT.mkdir(parents=True, exist_ok=True)

    if not args.skip_build:
        run('cargo', 'ndk', '-t', 'arm64-v8a', '-P', '26', 'build', '--locked', '-p', CRATE, '--lib', '--profile', 'quest', env=env)
    target = Path(json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=ROOT, text=True))['target_directory'])
    lib = target / 'aarch64-linux-android/quest' / f'lib{LIB}.so'
    if not lib.is_file():
        sys.exit(f'missing {lib}')
    host = 'darwin-x86_64' if os.uname().sysname == 'Darwin' else 'linux-x86_64'
    cxx = ndk / f'toolchains/llvm/prebuilt/{host}/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so'
    libs = OUT / 'lib/arm64-v8a'
    libs.mkdir(parents=True, exist_ok=True)
    # Symbols stripped: 55 MB instead of 70 (panics still name the place).
    run(ndk / f'toolchains/llvm/prebuilt/{host}/bin/llvm-strip', '-o', libs / lib.name, lib)
    shutil.copy(cxx, libs / cxx.name)

    # The host's bundle: what the browser fetches next to its page, here
    # read from the APK's assets/web (app::android::fetch).
    web = OUT / 'assets/web'
    if not args.skip_assets:
        shutil.rmtree(web, ignore_errors=True)
        cmd = [sys.executable, ROOT / 'engine/tools/build-web.py', '--warp', '--assets-only', web]
        if args.games: cmd += ['--games', args.games]
        run(*cmd)
    if args.lib_only:
        print(f'{libs} and {web}: ready for the app (mobile/android/app/src/main/jniLibs and assets)')
        return

    unsigned, aligned, apk = OUT / 'unsigned.apk', OUT / 'aligned.apk', OUT / 'splitscreen-host.apk'
    run(tools / 'aapt2', 'link', '-I', jar, '--manifest', MANIFEST, '-A', OUT / 'assets', '--version-code', args.version_code,
        '--version-name', f'0.1.{args.version_code}', '--replace-version', '-o', unsigned)
    with zipfile.ZipFile(unsigned, 'a', zipfile.ZIP_STORED) as z:
        for so in sorted(libs.iterdir()):
            z.write(so, f'lib/arm64-v8a/{so.name}')
    # Native libraries stay uncompressed and page aligned (extractNativeLibs=false).
    run(tools / 'zipalign', '-P', '16', '-f', '4', unsigned, aligned)
    if args.keystore:
        alias, password = os.environ.get('HOST_KEY_ALIAS', 'upload'), os.environ['HOST_KEYSTORE_PASSWORD']
        key = args.keystore
    else:
        key, alias, password = Path.home() / '.android/splitscreen-host-debug.keystore', 'androiddebugkey', 'android'
        key.parent.mkdir(exist_ok=True)
        if not key.exists():
            run('keytool', '-genkeypair', '-keystore', key, '-storepass', password, '-keypass', password, '-alias', alias,
                '-keyalg', 'RSA', '-keysize', '2048', '-validity', '10000', '-dname', 'CN=Splitscreen Club Host Debug')
    run(tools / 'apksigner', 'sign', '--ks', key, '--ks-key-alias', alias, '--ks-pass', f'pass:{password}', '--key-pass', f'pass:{password}', '--out', apk, aligned, env=env)
    for f in (unsigned, aligned): f.unlink(missing_ok=True)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT))
    (OUT / 'build.json').write_text(json.dumps({'revision': revision, 'dirty': dirty, 'version_code': args.version_code,
        'sha256': hashlib.sha256(apk.read_bytes()).hexdigest(), 'bytes': apk.stat().st_size}, indent=2) + '\n')
    print(f'{apk} ({apk.stat().st_size / 1e6:.1f} MB)')
    if args.install:
        run(sdk / 'platform-tools/adb', 'install', '-r', apk)


if __name__ == '__main__':
    main()
