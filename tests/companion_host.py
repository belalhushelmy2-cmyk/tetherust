"""Offline host interoperability test. Never invokes ADB or accesses an Android device."""
import os,pathlib,subprocess,tempfile,time
project=pathlib.Path(__file__).resolve().parents[1]
compiler=pathlib.Path(os.environ['KOTLIN_COMPILER_HOME'])
stdlib=next(compiler.glob('kotlin-stdlib-[0-9]*.jar'))
with tempfile.TemporaryDirectory(prefix='tetherust-companion-host-') as directory:
 root=pathlib.Path(directory);fixture=root/'fixture.json'
 subprocess.run(['keytool','-genkeypair','-storetype','PKCS12','-keystore',str(root/'test.p12'),'-alias','test','-keyalg','EC','-groupname','secp256r1','-validity','2','-storepass','test-password','-keypass','test-password','-dname','CN=Tetherust Test','-noprompt'],check=True,capture_output=True)
 subprocess.run(['java','-cp',str(compiler/'*'),'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler','-no-stdlib','-no-reflect','-jvm-target','1.8','-classpath',str(stdlib),'-d',str(root/'classes'),str(project/'android/src/org/tetherust/companion/ProxyEngine.kt'),str(project/'android/src/org/tetherust/companion/ForwardPath.kt'),str(project/'android/tests/JvmServer.kt'),str(project/'android/tests/ProxyChecks.kt')],check=True)
 process=subprocess.Popen(['java','-cp',str(root/'classes')+':'+str(compiler/'*'),'org.tetherust.companion.JvmServer',str(root/'test.p12'),str(fixture)],stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
 try:
  end=time.monotonic()+10
  while not fixture.exists():
   if process.poll() is not None:raise RuntimeError(process.stderr.read().decode())
   if time.monotonic()>end:raise TimeoutError('Host fixture startup')
   time.sleep(.1)
  os.chmod(fixture,0o600)
  env=dict(os.environ,TETHERUST_ADB_DISABLED='1',TETHERUST_PROXY_FIXTURE=str(fixture))
  test=['tun_namespace'] if os.environ.get('TETHERUST_KERNEL_TUN') else ['companion','kotlin_and_rust']
  subprocess.run(['cargo','test','--manifest-path',str(project/'Cargo.toml'),'--test',*test,'--','--ignored','--nocapture'],env=env,check=True)
 finally:
  process.stdin.close()
  try:process.wait(timeout=5)
  except subprocess.TimeoutExpired:process.kill();process.wait()
if os.environ.get('TETHERUST_KERNEL_TUN'):
 print('PASS: ordinary application TCP/UDP and virtual DNS through private kernel TUN and Kotlin TLS gateway; no phone accessed')
else:
 print('PASS: Kotlin/Rust TLS pinning, token rejection, TCP forwarding and UDP-over-TLS; no phone accessed')
