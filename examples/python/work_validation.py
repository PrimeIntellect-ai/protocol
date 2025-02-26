import socket
import time
import json
import os
import platform

def get_default_socket_path() -> str:
    """Returns the default socket path based on the operating system."""
    return "/tmp/com.prime.worker/metrics.sock" if platform.system() == "Darwin" else "/var/run/com.prime.worker/metrics.sock"

def send_message(metric: dict, socket_path: str = None) -> bool:
    socket_path = socket_path or os.getenv("PRIME_TASK_BRIDGE_SOCKET", get_default_socket_path())
    print("Sending message to socket: ", socket_path)

    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.connect(socket_path)
            message = json.dumps(metric) + "\n"
            sock.sendall(message.encode())
        return True
    except Exception as e:
        print(f"Failed to send message: {e}")
        return False

if __name__ == "__main__":
    file_sha = "b59ef2af5837d44a670ec6731cca834a533033c83a81dd308f1a55c5a45e4453" 
    file_name = "out_7bcd49e0-1e99-45a8-b9bb-ffb58f0f1f12.jsonl"

    if send_message({"file_sha": file_sha, "file_name": file_name}):
        print(f"Sent: {file_sha} {file_name}")
    else:
        print(f"Failed to send: {file_sha} {file_name}")