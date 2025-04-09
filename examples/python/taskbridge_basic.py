import socket
import time
import json
import os
import threading
import platform

def get_default_socket_path():
    """Returns the default socket path based on the operating system."""
    return "/tmp/com.prime.worker/metrics.sock" if platform.system() == "Darwin" else "/var/run/com.prime.worker/metrics.sock"

def send_message(metric, task_id=None):
    """Sends a message to the socket."""
    socket_path = get_default_socket_path()
    print(f"Thread {threading.current_thread().name}: Sending message to socket: {socket_path}")
    
    if task_id is None:
        task_id = "0725637c-ad20-4c30-b4e2-90cdf63b9974"
    
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.connect(socket_path)
            msg_buffer = []
            for key, value in metric.items():
                msg_buffer.append(json.dumps({"label": key, "value": value, "task_id": task_id}))
            print(f"Thread {threading.current_thread().name}: Sending {len(msg_buffer)} objects")
            sock.sendall(("\n".join(msg_buffer)).encode())
            # Close immediately after sending (like in production)
        return True
    except Exception as e:
        print(f"Thread {threading.current_thread().name}: Failed to send message: {e}")
        return False

def send_metrics():
    """Simulates sending metrics data."""
    metrics = {
        "cpu_percent": 25.5,
        "memory_percent": 60.2,
        "memory_usage": 14323642368,
        "memory_total": 507204362240,
        "gpu_0_memory_used": 76547358720,
        "gpu_0_memory_total": 85899345920,
        "gpu_0_utilization": 67,
        "gpu_1_memory_used": 76715130880,
        "gpu_1_memory_total": 85899345920,
        "gpu_1_utilization": 80,
        "gpu_2_memory_used": 76715130880,
        "gpu_2_memory_total": 85899345920,
        "gpu_2_utilization": 90,
        "gpu_3_memory_used": 76715130880,
        "gpu_3_memory_total": 85899345920,
        "gpu_3_utilization": 91,
    }
    send_message(metrics)

def send_file_info():
    """Simulates sending file info."""
    file_data = {
        "file_name": "test_file.txt",
        "file_sha": "20638f48221b266635376b399254d0faf17f69da567fd0f5deb3d6775c7c7607"
    }
    send_message(file_data)

if __name__ == "__main__":
    # Start multiple threads to simulate parallel sending
    threads = []
    
    # Create metric sending thread
    metric_thread = threading.Thread(target=send_metrics, name="Metrics")
    threads.append(metric_thread)
    
    # Create file info thread
    file_thread = threading.Thread(target=send_file_info, name="FileInfo")
    threads.append(file_thread)
    
    # Start all threads
    for thread in threads:
        thread.start()
        # Small delay to ensure they're not exactly synchronized
        time.sleep(0.01)
    
    # Wait for all threads to complete
    for thread in threads:
        thread.join()
    
    print("All messages sent!")