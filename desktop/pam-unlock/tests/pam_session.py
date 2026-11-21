#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Run as root in a disposable Linux-PAM container with Python 3 and OpenSSL.

    python3 pam_session.py /absolute/path/to/libpam_nectarlink_phone_unlock.so
"""

import contextlib
import ctypes as c
import json
import os
from pathlib import Path
import pwd
import socket
import subprocess as sp
import sys
import tempfile

SERVICE = "nectarlink-phone-lock"
SOCKET = "nectarlink-phone-unlock.sock"
ACCOUNT = pwd.getpwnam("nobody")
USER = dict(user=ACCOUNT.pw_uid, group=ACCOUNT.pw_gid, extra_groups=[])
SCRIPT = [sys.executable, str(Path(__file__).resolve())]
PAM_AUTH_ERR = 7


def authenticate(confdir, service, account, remote):
    pam = c.CDLL("libpam.so.0")
    handler = c.c_void_p()
    callback_type = c.CFUNCTYPE(c.c_int, c.c_int, c.c_void_p, c.c_void_p, c.c_void_p)

    @callback_type
    def fail_conversation(*_args):
        return 19  # PAM_CONV_ERR

    class Conversation(c.Structure):
        _fields_ = [("callback", callback_type), ("data", c.c_void_p)]

    conversation = Conversation(fail_conversation, None)
    result = pam.pam_start_confdir(
        service.encode(), account.encode(), c.byref(conversation), os.fsencode(confdir), c.byref(handler)
    )
    assert result == 0, f"pam_start_confdir returned {result}"
    if remote:
        assert pam.pam_set_item(handler, 4, remote.encode()) == 0  # PAM_RHOST
    result = pam.pam_authenticate(handler, 0)
    if result == 0:
        assert pam.pam_setcred(handler, 2) == 0  # PAM_ESTABLISH_CRED
    assert pam.pam_end(handler, result) == 0
    print(result)


def relay(path, attempts):
    with socket.socket(socket.AF_UNIX) as listener, contextlib.ExitStack() as connections:
        listener.bind(path)
        os.chmod(path, 0o600)
        listener.listen()
        listener.settimeout(8)
        print("ready", flush=True)
        requests = []
        for _ in range(int(attempts)):
            stream, _ = listener.accept()
            connections.enter_context(stream)
            stream.settimeout(8)
            requests.append((stream, stream.makefile("rb").read(1025)))
        print(json.dumps([json.loads(body) if body else None for _, body in requests]), flush=True)
        for stream, body in requests:
            if not body:
                continue
            response = sys.stdin.readline().strip()
            if response:
                stream.sendall(bytes.fromhex(response))
            stream.shutdown(socket.SHUT_WR)


def exercise(module):
    assert os.geteuid() == 0
    assert module.is_file()
    with tempfile.TemporaryDirectory(dir="/run", prefix="nectarlink-pam-test-") as directory:
        root = Path(directory)
        root.chmod(0o755)
        runtime = root / "runtime"
        runtime.mkdir(mode=0o700)
        os.chown(runtime, ACCOUNT.pw_uid, ACCOUNT.pw_gid)
        store = root / "store"
        store.mkdir(mode=0o755)
        conf = root / "pam.d"
        conf.mkdir(mode=0o755)
        test_key = root / "phone.key"
        sp.run(["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", test_key], check=True)
        point = sp.check_output(["openssl", "pkey", "-in", test_key, "-pubout", "-outform", "DER"])[-65:]
        assert point[0] == 4  # The final bytes of this SPKI are the uncompressed EC point.
        enrollment = dict(pc=[1] * 32, phone=[2] * 32, uid=ACCOUNT.pw_uid, public_key=list(point))
        record = store / f"{ACCOUNT.pw_uid}.json"
        record.write_text(json.dumps(enrollment))
        record.chmod(0o644)
        stack = f"auth required {module} enrollment_dir={store} runtime_dir={runtime}\n"
        (conf / SERVICE).write_text(stack)
        (conf / "wrong-service").write_text(stack)
        socket_path = runtime / SOCKET

        def pam(service=SERVICE, account="nobody", remote=""):
            return sp.Popen(SCRIPT + ["auth", str(conf), service, account, remote], stdout=sp.PIPE, text=True, **USER)

        def result(process, expected):
            output, _ = process.communicate(timeout=10)
            assert process.returncode == 0
            assert output.strip() == str(expected), (output, expected)

        def sign(request):
            assert request["version"] == 1 and request["operation"] == "sign"
            challenge = request["challenge"]
            assert challenge["pc"] == enrollment["pc"]
            assert challenge["phone"] == enrollment["phone"]
            assert challenge["uid"] == enrollment["uid"]
            assert len(challenge["nonce"]) == 32
            transcript = (
                b"nectarlink/linux-unlock/v1\0" + bytes(challenge["pc"]) + bytes(challenge["phone"])
                + challenge["uid"].to_bytes(4, "big") + bytes(challenge["nonce"])
            )
            signature = sp.check_output(["openssl", "dgst", "-sha256", "-sign", test_key], input=transcript)
            return json.dumps(dict(version=1, signature=list(signature))).encode()

        @contextlib.contextmanager
        def broker(attempts=1, credentials=USER):
            with sp.Popen(
                SCRIPT + ["relay", str(socket_path), str(attempts)],
                stdin=sp.PIPE, stdout=sp.PIPE, text=True, **credentials
            ) as server:
                try:
                    assert server.stdout.readline().strip() == "ready"
                    yield server
                finally:
                    if server.poll() is None:
                        server.kill()
                    socket_path.unlink(missing_ok=True)

        def send(server, *payloads):
            server.stdin.write("".join(payload.hex() + "\n" for payload in payloads))
            server.stdin.flush()
            assert server.wait(timeout=10) == 0

        result(pam(), PAM_AUTH_ERR)  # Relay missing.
        with broker(2) as server:
            clients = [pam(), pam()]
            first, second = json.loads(server.stdout.readline())
            assert first["challenge"]["nonce"] != second["challenge"]["nonce"]
            reply = sign(first)
            send(server, reply, reply)
            outcomes = [int(client.communicate(timeout=10)[0]) for client in clients]
            assert sorted(outcomes) == [0, PAM_AUTH_ERR], outcomes
        with broker() as server:
            client = pam()
            json.loads(server.stdout.readline())
            send(server, reply)
            result(client, PAM_AUTH_ERR)  # Same signed response in a later transaction.
        for malformed in [b'{"success":true}', b'{"version":1,"signature":', b"", b" " * 1025]:
            with broker() as server:
                client = pam()
                json.loads(server.stdout.readline())
                send(server, malformed)
                result(client, PAM_AUTH_ERR)
        for revoke in (False, True):
            with broker() as server:
                client = pam()
                request, = json.loads(server.stdout.readline())
                if revoke:
                    record.unlink()
                else:
                    replacement = store / "replacement"
                    replacement.write_bytes(record.read_bytes())
                    replacement.chmod(0o644)
                    replacement.replace(record)
                send(server, sign(request))
                result(client, PAM_AUTH_ERR)
            record.write_text(json.dumps(enrollment))
            record.chmod(0o644)
        with broker(credentials={}) as server:
            os.chown(socket_path, ACCOUNT.pw_uid, ACCOUNT.pw_gid)
            client = pam()
            assert json.loads(server.stdout.readline()) == [None]
            result(client, PAM_AUTH_ERR)  # Filesystem owner and actual peer differ.
        with broker():
            result(pam(service="wrong-service"), PAM_AUTH_ERR)
            result(pam(account="daemon"), PAM_AUTH_ERR)
            result(pam(remote="remote.invalid"), PAM_AUTH_ERR)
            record.chmod(0o666)
            result(pam(), PAM_AUTH_ERR)
            record.chmod(0o644)
            target = store / "target"
            record.rename(target)
            record.symlink_to(target)
            result(pam(), PAM_AUTH_ERR)
            record.unlink()
            target.rename(record)
            alternate_socket = runtime / "alternate"
            socket_path.rename(alternate_socket)
            socket_path.symlink_to(alternate_socket)
            result(pam(), PAM_AUTH_ERR)
            alternate_socket.unlink()
        print("Temporary PAM service accepted one fresh signature and rejected replay, malformed replies, unavailable relay, revoked or replaced enrollment, and mismatched bindings.")


if __name__ == "__main__":
    if sys.argv[1] == "auth":
        authenticate(*sys.argv[2:])
    elif sys.argv[1] == "relay":
        relay(*sys.argv[2:])
    else:
        exercise(Path(sys.argv[1]).resolve())
