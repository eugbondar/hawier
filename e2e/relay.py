"""One guest connection to the node under test (spec: device-e2e).

QEMU runs this once per TCP connection a device opens to 10.0.2.100:80
(`guestfwd=...-cmd:`): stdin/stdout are the guest's side, the socket is the
node's. The chardev form of guestfwd (`-tcp:host:port`) would put every guest
connection onto one host stream — invisible while each response is read to
the end, and garbage the moment a device aborts a download halfway.
"""

import os
import socket
import sys
import threading

node = socket.create_connection((sys.argv[1], int(sys.argv[2])))


def guest_to_node():
    while True:
        chunk = os.read(0, 65536)
        if not chunk:
            break
        node.sendall(chunk)
    try:
        node.shutdown(socket.SHUT_WR)
    except OSError:
        pass


threading.Thread(target=guest_to_node, daemon=True).start()
while True:
    chunk = node.recv(65536)
    if not chunk:
        break
    view = memoryview(chunk)
    while view:
        view = view[os.write(1, view):]
