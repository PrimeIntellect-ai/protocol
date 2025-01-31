import socket
import time
import json
import os
import platform

def get_default_socket_path() -> str:
    """Returns the default socket path based on the operating system."""
    return "/tmp/com.prime.miner/metrics.sock" if platform.system() == "Darwin" else "/var/run/com.prime.miner/metrics.sock"

def send_message(metric: dict, socket_path: str = None) -> bool:
    """Sends a message to the specified socket path or uses the default if none is provided."""
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
    for i in range(3):
        metric = {"label": "progress", "value": i * 100, "task_id": "df64d2ea-1ec4-4cbb-9342-40e39c2ac89b"}

        if send_message(metric):
            print(f"Sent: {metric}")
        time.sleep(1)