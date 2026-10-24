"""Development-only archive/install smoke checks. No root or system installation."""
import os, pathlib, subprocess, tarfile, tempfile
project=pathlib.Path(__file__).resolve().parents[1]
dist=project/'dist'
with tempfile.TemporaryDirectory(prefix='tetherust-packages-') as directory:
    root=pathlib.Path(directory)
    with tarfile.open(dist/'tetherust-0.3.0-linux-x86_64.tar.gz') as archive:
        archive.extractall(root, filter='data')
    bundle=root/'tetherust-0.3.0'; prefix=root/'install space $ and %'
    env=dict(os.environ,PREFIX=str(prefix))
    subprocess.run([str(bundle/'install.sh')],env=env,check=True,capture_output=True)
    version=subprocess.check_output([str(prefix/'bin/tetherust'),'--version'],text=True)
    assert version.strip()=='tetherust 0.3.0'
    assert (prefix/'lib/tetherust/tetherust-netguard').read_bytes()==(bundle/'lib/tetherust/tetherust-netguard').read_bytes()
    desktop=prefix/'share/applications/tetherust.desktop'
    subprocess.run(['desktop-file-validate',str(desktop)],check=True,capture_output=True)
    assert '\\$' in desktop.read_text() and '%%' in desktop.read_text()
    subprocess.run([str(bundle/'uninstall.sh')],env=env,check=True,capture_output=True)
    assert not (prefix/'bin/tetherust').exists() and not desktop.exists()
    assert not (prefix/'lib/tetherust/tetherust-netguard').exists()
    deb=dist/'tetherust_0.3.0-1_amd64.deb'
    members=subprocess.check_output(['ar','t',str(deb)],text=True).splitlines()
    assert members==['debian-binary','control.tar.gz','data.tar.gz']
    assert subprocess.check_output(['ar','p',str(deb),'debian-binary'])==b'2.0\n'
    (root/'control.tar.gz').write_bytes(subprocess.check_output(['ar','p',str(deb),'control.tar.gz']))
    with tarfile.open(root/'control.tar.gz') as archive:
        control=archive.extractfile('./control').read().decode()
        assert 'Architecture: amd64' in control and 'network-manager' in control and 'libc6 (>=' in control
    (root/'data.tar.gz').write_bytes(subprocess.check_output(['ar','p',str(deb),'data.tar.gz']))
    with tarfile.open(root/'data.tar.gz') as archive:
        binary=archive.extractfile('./usr/bin/tetherust').read()
        assert binary==(bundle/'bin/tetherust').read_bytes()
        assert archive.extractfile('./usr/share/doc/tetherust/THIRD-PARTY-NOTICES.txt').read()
        assert archive.extractfile('./usr/lib/tetherust/tetherust-netguard').read()==(bundle/'lib/tetherust/tetherust-netguard').read_bytes()
    appimage=dist/'Tetherust-0.3.0-x86_64.AppImage'
    result=subprocess.check_output([str(appimage),'--version'],env=dict(os.environ,APPIMAGE_EXTRACT_AND_RUN='1'),text=True)
    assert result.strip()=='tetherust 0.3.0'
    subprocess.run([str(appimage),'--appimage-extract'],cwd=root,check=True,stdout=subprocess.DEVNULL)
    subprocess.run(['python3',str(project/'tests/native_sway.py')],env=dict(os.environ,TETHERUST_TEST_BINARY=str(root/'squashfs-root/AppRun')),check=True)
    subprocess.run(['sha256sum','--check','SHA256SUMS'],cwd=dist,check=True)
print('PASS: tar install/uninstall with special-character prefix, DEB structure/ABI metadata/payload identity, AppImage extraction/runtime CLI and bundled Wayland UI')
