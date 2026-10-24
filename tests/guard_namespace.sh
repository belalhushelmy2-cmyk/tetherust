#!/usr/bin/env bash
# A private user + network + mount namespace only. Never touches the host firewall.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ ${1:-} != inside ]]; then
    export TETHERUST_HOST_NETNS=$(readlink /proc/self/ns/net)
    export TETHERUST_GUARD_BINARY=${TETHERUST_GUARD_BINARY:-$(realpath ../target/debug/tetherust-netguard)}
    exec unshare --user --map-root-user --net --mount "$0" inside
fi
[[ $(readlink /proc/self/ns/net) != "$TETHERUST_HOST_NETNS" ]]
mount --make-rprivate /
mount -t tmpfs tmpfs /run
ip link set lo up
peer_pid=
echo_pid=
cleanup() { if [[ -n $echo_pid ]]; then kill "$echo_pid" 2>/dev/null || true; wait "$echo_pid" 2>/dev/null || true; fi; if [[ -n $peer_pid ]]; then kill "$peer_pid" 2>/dev/null || true; wait "$peer_pid" 2>/dev/null || true; fi; }
trap cleanup EXIT
unshare --net sleep 120 & peer_pid=$!
# Child namespace creation is asynchronous; wait for its namespace to differ.
for _ in {1..100}; do [[ $(readlink "/proc/$peer_pid/ns/net") != $(readlink /proc/self/ns/net) ]] && break; sleep .01; done
ip link add guard-test type veth peer name guard-peer
ip link set guard-peer netns "$peer_pid"
ip addr add 10.23.0.1/24 dev guard-test
ip -6 addr add fd23::1/64 dev guard-test nodad
ip link set guard-test up
nsenter -t "$peer_pid" -n ip link set lo up
nsenter -t "$peer_pid" -n ip link set guard-peer up
nsenter -t "$peer_pid" -n ip addr add 10.23.0.2/24 dev guard-peer
nsenter -t "$peer_pid" -n ip addr add 203.0.113.2/24 dev guard-peer
nsenter -t "$peer_pid" -n ip -6 addr add fd23::2/64 dev guard-peer nodad
nsenter -t "$peer_pid" -n ip -6 addr add 2001:db8:23::2/64 dev guard-peer nodad
ip route add 203.0.113.0/24 via 10.23.0.2
ip -6 route add 2001:db8:23::/64 via fd23::2
ping -c1 -W1 203.0.113.2 >/dev/null
ping -6 -c1 -W1 2001:db8:23::2 >/dev/null
nsenter -t "$peer_pid" -n python3 - <<'PEER' &
import socket,threading,time
for port in [53,443]:
 def tcp(port=port):
  server=socket.socket();server.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);server.bind(('0.0.0.0',port));server.listen()
  def echo(client):
   with client:
    try:
     while data:=client.recv(1024):client.sendall(data)
    except OSError:pass
  while True:threading.Thread(target=echo,args=(server.accept()[0],),daemon=True).start()
 def udp(port=port,address="10.23.0.2"):
  server=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);server.bind((address,port))
  while True:
   data,peer=server.recvfrom(1024);server.sendto(data,peer)
 threading.Thread(target=tcp,daemon=True).start()
 for address in ["10.23.0.2","203.0.113.2"]:threading.Thread(target=udp,kwargs={"address":address},daemon=True).start()
while True:time.sleep(60)
PEER
echo_pid=$!
python3 - "$TETHERUST_GUARD_BINARY" <<'CHECK'
import os,socket,subprocess,sys,time
helper=sys.argv[1]
def guard(action):subprocess.run([helper,action],env=dict(os.environ,PKEXEC_UID='1000'),check=True)
def ping(ip,success):
 result=subprocess.run(['ping','-c1','-W1',ip],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 assert (result.returncode==0)==success,ip
def echo(ip,port,udp=False):
 with socket.socket(socket.AF_INET,socket.SOCK_DGRAM if udp else socket.SOCK_STREAM) as client:
  client.settimeout(1);client.connect((ip,port));client.sendall(b'test');assert client.recv(4)==b'test'
def blocked(ip,port,udp=False):
 try:echo(ip,port,udp)
 except (TimeoutError,OSError):return
 raise AssertionError(('leak',ip,port,udp))
for _ in range(100):
 try:echo('10.23.0.2',443);break
 except OSError:time.sleep(.02)
else:raise AssertionError('peer did not start')
old=socket.create_connection(('203.0.113.2',443));old.settimeout(.3);old.sendall(b'test');assert old.recv(4)==b'test'
guard('enable')
assert open('/run/tetherust-netguard/active').read().strip()=='enabled'
assert os.stat('/run/tetherust-netguard/active').st_mode&0o777==0o644
ping('127.0.0.1',True);ping('10.23.0.2',True);ping('203.0.113.2',False);ping('2001:db8:23::2',False)
echo('10.23.0.2',443);echo('10.23.0.2',443,True)
for udp in [False,True]:blocked('10.23.0.2',53,udp);blocked('203.0.113.2',443,udp)
old.sendall(b'fail')
try:assert not old.recv(4)
except (TimeoutError,OSError):pass
else:raise AssertionError('existing connection bypassed guard')
old.close()
subprocess.run(['ip','link','set','guard-test','name','tetherust0'],check=True)
ping('203.0.113.2',True);ping('2001:db8:23::2',True)
echo('203.0.113.2',443)
echo('203.0.113.2',443,True)
subprocess.run(['ip','link','set','tetherust0','name','guard-test'],check=True)
ping('203.0.113.2',False)
subprocess.run(['nft','add','table','inet','unrelated_fixture'],check=True)
guard('disable')
assert not os.path.exists('/run/tetherust-netguard/active')
subprocess.run(['nft','list','table','inet','unrelated_fixture'],check=True,stdout=subprocess.DEVNULL)
ping('203.0.113.2',True);ping('2001:db8:23::2',True);echo('10.23.0.2',53);echo('10.23.0.2',53,True)
print('PASS: private guard blocks TCP/UDP/DNS/IPv4/IPv6 and existing connections, allows TUN/LAN/loopback, explicit recovery preserves unrelated table; host untouched')
CHECK
