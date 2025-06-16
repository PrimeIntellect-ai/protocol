import socket
import time
import json
import os
import threading
import platform

def get_default_socket_path():
    """Returns the default socket path based on the operating system."""
    return "/tmp/com.prime.worker/metrics.sock" if platform.system() == "Darwin" else "/var/run/com.prime.worker/metrics.sock"

def send_message(metrics, task_id=None):
    """Sends a message to the socket."""
    socket_path = get_default_socket_path()
    print(f"Thread {threading.current_thread().name}: Sending message to socket: {socket_path}")
    
    if task_id is None:
        task_id = "ac05530d-6f13-4b7b-9f18-36de020d71b3"
    metrics["task_id"] = task_id
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.connect(socket_path)
            # Send metrics directly as JSON
            message = json.dumps(metrics)
            sock.sendall(message.encode())
        return True
    except Exception as e:
        print(f"Thread {threading.current_thread().name}: Failed to send message: {e}")
        return False

def send_metrics():
    """Simulates sending metrics data."""
    metrics = {
        "system/cpu_percent": 25.5,
        "system/memory_percent": 60.2,
        "system/memory_usage": 14323642368,
        "system/memory_total": 507204362240,
        "system/gpu_0_memory_used": 76547358720,
        "system/gpu_0_memory_total": 85899345920,
        "system/gpu_0_utilization": 67,
        "system/gpu_1_memory_used": 76715130880,
        "system/gpu_1_memory_total": 85899345920,
        "system/gpu_1_utilization": 80,
        "system/gpu_2_memory_used": 76715130880,
        "system/gpu_2_memory_total": 85899345920,
        "system/gpu_2_utilization": 90,
        "system/gpu_3_memory_used": 76715130880,
        "system/gpu_3_memory_total": 85899345920,
        "system/gpu_3_utilization": 91,
    }
    send_message(metrics)

def send_file_info():
    """Simulates sending file info.

    To simulate a submission with groups, simply create a file in s3/mappings.
    Sample: 
    - Filename: 1c4970c5032ba367efc537054d9d0f4595b8a7fe0901e8aa1b5c34c0c582ce84 
    - Content for a group of size 2: Qwen0.6/PrimeIntellect/INTELLECT-2-RL-Dataset/8-18084400840775688168-2-0-0.parquet
    """
    file_data = {
        "output/save_path": "/path/to/save/file.txt",
        "output/sha256": "1c4970c5032ba367efc537054d9d0f4595b8a7fe0901e8aa1b5c34c0c582ce84",
        "output/output_flops": 1500000,
        "output/input_flops": 500000
    }
    send_message(file_data)

if __name__ == "__main__":
    # Start multiple threads to simulate parallel sending
    threads = []
    
    # Create metric sending thread
    #metric_thread = threading.Thread(target=send_metrics, name="Metrics")
    #threads.append(metric_thread)
    
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