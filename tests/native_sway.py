"""Read-only network / isolated-file GUI acceptance on a private headless Sway.
Development-only Python harness. Does not connect any live network or change the user's config.
Usage: TETHERUST_TEST_BINARY=/absolute/tetherust python3 tests/native_sway.py
"""
import json, os, pathlib, subprocess, tempfile, time
project = pathlib.Path(__file__).resolve().parents[1]
binary = pathlib.Path(os.environ.get('TETHERUST_TEST_BINARY', project.parent / 'target/debug/tetherust'))
output = project / 'dist'; output.mkdir(exist_ok=True)
def wait(check, seconds=12):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if check(): return
        time.sleep(.1)
    raise AssertionError('Acceptance condition timed out')
with tempfile.TemporaryDirectory(prefix='tetherust-native-') as directory:
    root = pathlib.Path(directory); runtime = root / 'run'; runtime.mkdir(mode=0o700)
    config = root / 'sway.conf'
    config.write_text('output HEADLESS-1 resolution 1280x900\nseat seat0 fallback true\ndefault_floating_border none\nfor_window [title="^Tetherust$"] floating enable\n')
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), XDG_DATA_HOME=str(root/'data'), XDG_CONFIG_HOME=str(root/'config'), WLR_BACKENDS='headless', WLR_RENDERER='pixman', WLR_HEADLESS_OUTPUTS='1', WLR_LIBINPUT_NO_DEVICES='1', XDG_CURRENT_DESKTOP='sway', XDG_SESSION_TYPE='wayland', SLINT_BACKEND='winit-software', TETHERUST_ADB_DISABLED='1')
    for key in ['DISPLAY', 'WAYLAND_DISPLAY', 'SWAYSOCK', 'XDG_ACTIVATION_TOKEN']: env.pop(key, None)
    # The deprecated Sway cursor IPC may not deliver input on seats without a physical pointer.
    # Use the compositor's actual virtual-pointer protocol in this isolated display.
    protocol = next((pathlib.Path.home()/'.cargo/registry/src').glob('*/wayland-protocols-wlr-*/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml'))
    subprocess.run(['wayland-scanner','client-header',str(protocol),str(root/'virtual-pointer.h')],check=True)
    subprocess.run(['wayland-scanner','private-code',str(protocol),str(root/'virtual-pointer.c')],check=True)
    flags = subprocess.check_output(['pkg-config','--cflags','--libs','wayland-client'],text=True).split()
    subprocess.run(['cc',str(project/'tests/wayland_pointer.c'),str(root/'virtual-pointer.c'),'-I'+str(root),'-o',str(root/'pointer'),*flags],check=True)
    log = open(root/'log', 'w'); app = None
    compositor = subprocess.Popen(['sway', '--config', str(config)], env=env, stdout=log, stderr=log)
    try:
        wait(lambda: (runtime/'wayland-1').exists())
        env.update(WAYLAND_DISPLAY='wayland-1', SWAYSOCK=str(next(runtime.glob('sway-ipc.*.sock'))))
        app = subprocess.Popen([str(binary), 'gui'], env=env, stdout=log, stderr=log)
        def nodes(node): return [node] + [item for key in ['nodes','floating_nodes'] for child in node.get(key,[]) for item in nodes(child)]
        def window():
            tree = json.loads(subprocess.check_output(['swaymsg', '-s', env['SWAYSOCK'], '-t', 'get_tree'], env=env))
            return next((n for n in nodes(tree) if n.get('pid') == app.pid), None)
        wait(lambda: window() is not None)
        subprocess.run(['swaymsg', '-s', env['SWAYSOCK'], f'[pid={app.pid}] border none'], env=env, check=True, capture_output=True)
        time.sleep(.4)
        rect = window()['rect']; time.sleep(1.2)
        def click(x,y):
            subprocess.run([str(root/'pointer'),str(rect['x']+x),str(rect['y']+y),'1280','900'],env=env,check=True,timeout=5)
            time.sleep(.3)
        def capture(name):
            subprocess.run(['grim', '-g', f'{rect["x"]},{rect["y"]} {rect["width"]}x{rect["height"]}', str(output/name)], env=env, check=True)
        capture('dashboard.png')
        click(100,192); capture('devices.png')
        click(100,245); capture('settings.png')
        # Change safe user preferences, enable isolated autostart, then save.
        click(264,265); click(264,302); click(264,386); click(264,422)
        click(264,520); click(264,556)
        click(600,665)
        config_path = root/'data/tetherust/config.json'
        # Button coordinates can change with toolkit metrics. Test fails rather than pretending a click worked.
        wait(lambda: config_path.exists(), seconds=3)
        saved = json.loads(config_path.read_text()); assert saved['auto_connect'] is False and saved['full_system'] is True
        assert saved['require_phone_vpn'] is True and saved['kill_switch'] is True
        assert saved['auto_reconnect'] is False and saved['notifications'] is False
        assert (root/'config/autostart/tetherust.desktop').is_file()
        click(100,299); capture('logs.png')
        click(940,116)
        wait(lambda: bool(list((root/'data/tetherust').glob('tetherust-logs-*.txt'))), seconds=3)
        exported = next((root/'data/tetherust').glob('tetherust-logs-*.txt')).read_text()
        assert 'network device inventory changed' in exported
        assert '192.168.' not in exported and 'public_ip' not in exported
        click(833,118)
        copied = subprocess.check_output(['wl-paste','--no-newline'],env=env,text=True,timeout=4)
        assert 'network device inventory changed' in copied and '192.168.' not in copied
        click(244,118); capture('advanced-logs.png')
        # A second GUI session must reject the same user-data lock.
        second = subprocess.run([str(binary),'gui'], env=env, capture_output=True, text=True, timeout=5)
        assert second.returncode != 0 and 'Another Tetherust' in second.stderr
        ticks = os.sysconf('SC_CLK_TCK'); page_size = os.sysconf('SC_PAGE_SIZE')
        def cpu_ticks():
            fields = pathlib.Path(f'/proc/{app.pid}/stat').read_text().split(); return int(fields[13])+int(fields[14])
        first = cpu_ticks(); before = time.monotonic(); time.sleep(3)
        cpu = (cpu_ticks()-first)/ticks/(time.monotonic()-before)*100
        rss = int(pathlib.Path(f'/proc/{app.pid}/statm').read_text().split()[1])*page_size
        print(f'Observed disconnected idle (3-second sample): CPU={cpu:.2f}% of one core, RSS={rss/1_000_000:.2f} MB')
        app.terminate(); assert app.wait(timeout=35) == 0
        print('PASS: private Wayland GUI, four pages, safe settings persistence, redacted log copy/export, advanced diagnostics, duplicate-instance rejection and SIGTERM shutdown')
    except Exception:
        log.flush(); print((root/'log').read_text()); raise
    finally:
        if app and app.poll() is None:
            app.terminate()
            try: app.wait(timeout=10)
            except subprocess.TimeoutExpired: app.kill(); app.wait(timeout=5)
        compositor.terminate(); compositor.wait(timeout=5); log.close()
