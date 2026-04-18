#!/usr/bin/env python3
"""Debug SSH exchange hash mismatch with VeerOS server."""
import socket
import struct
import hashlib
import os

def put_string(data: bytes) -> bytes:
    return struct.pack('>I', len(data)) + data

def put_mpint(val: bytes) -> bytes:
    # Strip leading zeros
    i = 0
    while i < len(val) and val[i] == 0:
        i += 1
    if i == len(val):
        return struct.pack('>I', 0)
    val = val[i:]
    if val[0] & 0x80:
        val = b'\x00' + val
    return struct.pack('>I', len(val)) + val

def get_string(data: bytes, offset: int) -> tuple:
    length = struct.unpack('>I', data[offset:offset+4])[0]
    return data[offset+4:offset+4+length], offset+4+length

def read_exact(sock, n):
    buf = b''
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise EOFError("Connection closed")
        buf += chunk
    return buf

def read_packet(sock):
    hdr = read_exact(sock, 4)
    packet_length = struct.unpack('>I', hdr)[0]
    body = read_exact(sock, packet_length)
    padding_length = body[0]
    payload = body[1:packet_length - padding_length]
    return payload

def write_packet(sock, payload):
    block_size = 8
    min_packet = 1 + len(payload) + 4
    padding_len = block_size - (min_packet % block_size)
    if padding_len < 4:
        padding_len += block_size
    packet_length = 1 + len(payload) + padding_len
    hdr = struct.pack('>I', packet_length)
    pad_byte = bytes([padding_len])
    padding = os.urandom(padding_len)
    sock.sendall(hdr + pad_byte + payload + padding)

def main():
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.settimeout(30)
    sock.connect(('127.0.0.1', 2223))

    # Version exchange
    server_version_line = b''
    while True:
        b = sock.recv(1)
        if not b:
            break
        server_version_line += b
        if b == b'\n':
            break

    v_s = server_version_line.rstrip(b'\r\n')
    print(f"V_S: {v_s}")

    v_c = b"SSH-2.0-DebugClient_1.0"
    sock.sendall(v_c + b"\r\n")
    print(f"V_C: {v_c}")

    # Build our KEXINIT (minimal) — send immediately after version
    i_c = bytearray()
    i_c.append(20)  # SSH_MSG_KEXINIT
    i_c.extend(os.urandom(16))  # cookie
    # name-lists
    for nl in [
        b"curve25519-sha256",  # kex
        b"ssh-ed25519",        # host key
        b"chacha20-poly1305@openssh.com",  # cipher c2s
        b"chacha20-poly1305@openssh.com",  # cipher s2c
        b"",   # mac c2s (implicit with chacha20-poly1305)
        b"",   # mac s2c
        b"none",  # compress c2s
        b"none",  # compress s2c
        b"",   # lang c2s
        b"",   # lang s2c
    ]:
        i_c.extend(struct.pack('>I', len(nl)))
        i_c.extend(nl)
    i_c.append(0)  # first_kex_packet_follows
    i_c.extend(struct.pack('>I', 0))  # reserved
    i_c = bytes(i_c)
    write_packet(sock, i_c)
    print(f"I_C length: {len(i_c)} (msg type: {i_c[0]})")

    # Now read server KEXINIT
    i_s = read_packet(sock)
    print(f"I_S length: {len(i_s)} (msg type: {i_s[0]})")
    print(f"I_S hex: {i_s.hex()}")
    print(f"I_C hex: {i_c.hex()}")

    # Send ECDH_INIT with our ephemeral key
    # Generate X25519 keypair
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
    eph_sk = X25519PrivateKey.generate()
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
    q_c = eph_sk.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    print(f"Q_C: {q_c.hex()}")

    ecdh_init = bytes([30]) + put_string(q_c)
    write_packet(sock, ecdh_init)

    # Read ECDH_REPLY
    reply = read_packet(sock)
    print(f"\nECDH_REPLY (type {reply[0]}):")
    off = 1

    # K_S blob
    k_s_blob, off = get_string(reply, off)
    print(f"K_S blob ({len(k_s_blob)} bytes): {k_s_blob.hex()}")

    # Parse K_S to get pubkey
    ks_algo, ks_off = get_string(k_s_blob, 0)
    ks_pubkey, _ = get_string(k_s_blob, ks_off)
    print(f"  algo: {ks_algo}")
    print(f"  pubkey: {ks_pubkey.hex()}")

    # Q_S
    q_s, off = get_string(reply, off)
    print(f"Q_S: {q_s.hex()}")

    # Signature blob
    sig_blob, off = get_string(reply, off)
    sig_algo, sig_off = get_string(sig_blob, 0)
    sig_bytes, _ = get_string(sig_blob, sig_off)
    print(f"Sig algo: {sig_algo}")
    print(f"Sig ({len(sig_bytes)} bytes): {sig_bytes.hex()}")

    # Compute shared secret
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PublicKey
    peer_pub = X25519PublicKey.from_public_bytes(q_s)
    shared_secret_raw = eph_sk.exchange(peer_pub)
    print(f"\nShared secret (raw): {shared_secret_raw.hex()}")

    # Compute exchange hash
    h = hashlib.sha256()
    h.update(put_string(v_c))
    h.update(put_string(v_s))
    h.update(put_string(i_c))
    h.update(put_string(i_s))
    h.update(put_string(k_s_blob))
    h.update(put_string(q_c))
    h.update(put_string(q_s))
    h.update(put_mpint(shared_secret_raw))
    exchange_hash = h.digest()
    print(f"\nExchange hash H: {exchange_hash.hex()}")

    # Verify signature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    host_pub = Ed25519PublicKey.from_public_bytes(ks_pubkey)
    try:
        host_pub.verify(sig_bytes, exchange_hash)
        print("Signature VALID!")
    except Exception as e:
        print(f"Signature INVALID: {e}")

    # Also try with reversed shared secret (LE->BE)
    h2 = hashlib.sha256()
    h2.update(put_string(v_c))
    h2.update(put_string(v_s))
    h2.update(put_string(i_c))
    h2.update(put_string(i_s))
    h2.update(put_string(k_s_blob))
    h2.update(put_string(q_c))
    h2.update(put_string(q_s))
    h2.update(put_mpint(shared_secret_raw[::-1]))
    exchange_hash2 = h2.digest()
    print(f"Exchange hash H (reversed K): {exchange_hash2.hex()}")
    try:
        host_pub.verify(sig_bytes, exchange_hash2)
        print("Signature VALID with reversed K!")
    except Exception as e:
        print(f"Signature INVALID with reversed K: {e}")

    sock.close()

if __name__ == '__main__':
    main()
