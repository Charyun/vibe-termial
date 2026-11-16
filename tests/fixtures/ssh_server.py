"""Disposable loopback SSH server for the ignored native transport test.
Accepts only a freshly generated fixture key or a test-only password; config stays in root.
Requires existing paramiko and a POSIX sh (Git for Windows is sufficient).
"""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import threading
import time
import paramiko

root = Path(sys.argv[1]).resolve()
root.mkdir(parents=True, exist_ok=True)
project = root / "remote project ' quoted"
project.mkdir(exist_ok=True)
keyfile = root / "identity"
subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(keyfile)], check=True)
client_key = paramiko.Ed25519Key.from_private_key_file(str(keyfile))
server_key = paramiko.RSAKey.generate(2048)
sh = shutil.which("sh")
if not sh:
    raise RuntimeError("A POSIX sh is required for the fixture")
processes = []
transports = []
channels = []
audit_lock = threading.Lock()

def posix_path(path):
    if os.name == "nt":
        return "/" + path.drive[0].lower() + "/" + path.relative_to(path.anchor).as_posix()
    return str(path)

class Server(paramiko.ServerInterface):
    def get_allowed_auths(self, username):
        return "publickey,password"
    def check_auth_password(self, username, password):
        return paramiko.AUTH_SUCCESSFUL if username == "fixture" and password == "fixture-pass" else paramiko.AUTH_FAILED
    def check_auth_publickey(self, username, key):
        return paramiko.AUTH_SUCCESSFUL if username == "fixture" and key == client_key else paramiko.AUTH_FAILED
    def check_channel_request(self, kind, channel_id):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
    def check_channel_pty_request(self, *args):
        return True
    def check_channel_exec_request(self, channel, command):
        threading.Thread(target=execute, args=(channel, command.decode()), daemon=True).start()
        return True

def execute(channel, command):
    env = os.environ.copy()
    env["SHELL"] = sh
    home = root / "home"
    home.mkdir(exist_ok=True)
    env["HOME"] = posix_path(home)
    env.pop("ENV", None)
    env.pop("BASH_ENV", None)
    env["CHERE_INVOKING"] = "1"
    process = subprocess.Popen([sh, "-c", command], cwd=project, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    processes.append(process)
    with audit_lock:
        with (root / "commands.log").open("a", encoding="utf-8") as out:
            out.write(command + "\n")
    def feed():
        try:
            while process.poll() is None:
                data = channel.recv(4096)
                if not data:
                    break
                with audit_lock:
                    with (root / "input.log").open("ab") as out: out.write(data)
                process.stdin.write(data.replace(b"\r", b"\n"))
                process.stdin.flush()
        except (EOFError, OSError):
            pass
        finally:
            try: process.stdin.close()
            except OSError: pass
    if command.startswith("cd "):
        # Model a real remote shell prompt, after the client's tty setup.
        time.sleep(0.3)
        channel.sendall(b"__VIBE_SSH_READY__\r\n")
    threading.Thread(target=feed, daemon=True).start()
    try:
        while True:
            data = os.read(process.stdout.fileno(), 4096)
            if not data: break
            with audit_lock:
                with (root / "output.log").open("ab") as out: out.write(data)
            channel.sendall(data)
        channel.send_exit_status(process.wait())
    except (EOFError, OSError):
        if process.poll() is None: process.terminate()
    finally:
        channel.close()

listener = socket.socket()
listener.bind(("127.0.0.1", 0))
listener.listen(8)
listener.settimeout(0.2)
port = listener.getsockname()[1]
config = root / "config"
config.write_text('Host 127.0.0.1\n  UserKnownHostsFile "' + (root / "known_hosts").as_posix() + '"\n  StrictHostKeyChecking accept-new\n  IdentitiesOnly yes\n  IdentityFile none\n', encoding="utf-8")
(root / "ready.json").write_text(json.dumps({"port": port, "identity": str(keyfile), "config": str(config), "project": posix_path(project)}), encoding="utf-8")
try:
    while not (root / "stop").exists():
        try: connection, _ = listener.accept()
        except socket.timeout: continue
        def accept(connection):
            transport = paramiko.Transport(connection)
            transports.append(transport)
            transport.add_server_key(server_key)
            try:
                transport.start_server(server=Server())
                while transport.is_active() and not (root / "stop").exists():
                    channel = transport.accept(0.2)
                    if channel is not None: channels.append(channel)
            except (EOFError, OSError, paramiko.SSHException): pass
        threading.Thread(target=accept, args=(connection,), daemon=True).start()
finally:
    listener.close()
    for transport in transports: transport.close()
    for process in processes:
        if process.poll() is None:
            process.terminate()
            try: process.wait(timeout=2)
            except subprocess.TimeoutExpired: process.kill()
