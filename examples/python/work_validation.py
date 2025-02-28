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
    file_sha = "1fc3f572b3630f34a92dc6a26d8e677f208e6a2f2c1f8ae2036388dbe85f4fc2" 
    file_name = "out_7bcd49e0-1e99-45a8-b9bb-ffb58f0f1f12.jsonl"

    if send_message({"label": "file_name", "value": file_name, "task_id": "4063bbd7-c458-4cd3-b082-6c2ea8f0e46a"}):
        print(f"Sent: {file_name}")
    else:
        print(f"Failed to send: {file_name}")

    if send_message({"label": "file_sha", "value": file_sha, "task_id": "4063bbd7-c458-4cd3-b082-6c2ea8f0e46a"}):
        print(f"Sent: {file_sha}")
    else:
        print(f"Failed to send: {file_sha}")