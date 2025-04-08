import socket
import time
import json
import os
import platform

def get_default_socket_path() -> str:
    """Returns the default socket path based on the operating system."""
    return "/tmp/com.prime.worker/metrics.sock" if platform.system() == "Darwin" else "/var/run/com.prime.worker/metrics.sock"

def send_message(metric: dict, socket_path: str = None) -> bool:
    """Sends a message to the specified socket path or uses the default if none is provided."""
    socket_path = socket_path or os.getenv("PRIME_TASK_BRIDGE_SOCKET", get_default_socket_path())
    print("Sending message to socket: ", socket_path)

    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.connect(socket_path)
            message = json.dumps(metric)
            sock.sendall(message.encode())
        return True
    except Exception as e:
        print(f"Failed to send message: {e}")
        return False

if __name__ == "__main__":
    """
    You can get the task_id directly from the docker env. 
    The worker reports the metrics using the heartbeat api but only for the currently running task. 
    """
    task_id = "0725637c-ad20-4c30-b4e2-90cdf63b9974"
    for i in range(5):
        metric = {"label": "progress", "value": i * 100, "task_id": task_id}

        if send_message(metric):
            print(f"Sent: {metric}")
        time.sleep(1)