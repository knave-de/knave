#!/usr/bin/env python3
"""Measure desktop IPC resource use in an isolated nested Villain instance.

Run after `cargo build --release`, from a running Wayland desktop. No installed
socket or configuration is changed. Context switches are a wakeup proxy, not
an exact count of scheduler wakeups caused by desktop IPC.
"""
import json
import os
from pathlib import Path
import select
import socket
import statistics
import subprocess
import tempfile
import time

BINARY = Path(__file__).resolve().parents[1] / "target/release/villain"
DURATION = 5.0


def sample(pid):
    proc = Path(f"/proc/{pid}")
    stat = (proc / "stat").read_text().split(") ", 1)[1].split()
    status = dict(line.split(":", 1) for line in (proc / "status").read_text().splitlines() if ":" in line)
    switches = 0
    for task in (proc / "task").iterdir():
        try:
            switches += sum(int(line.split(":", 1)[1]) for line in (task / "status").read_text().splitlines() if "ctxt_switches:" in line)
        except FileNotFoundError:
            pass
    return {"cpu_ticks": int(stat[11]) + int(stat[12]), "rss_kib": int(status["VmRSS"].split()[0]),
            "threads": int(status["Threads"]), "fds": len(list((proc / "fd").iterdir())),
            "context_switches": switches}


def request(file, data):
    file.write(json.dumps(data).encode() + b"\n")
    file.flush()
    line = file.readline()
    if not line:
        raise RuntimeError("desktop IPC disconnected")
    return json.loads(line)


def session(mode):
    with tempfile.TemporaryDirectory(prefix="knave-ipc-metrics-") as root:
        runtime = Path(root) / "runtime"
        runtime.mkdir(mode=0o700)
        host_runtime = Path(os.environ["XDG_RUNTIME_DIR"])
        host_display = Path(os.environ["WAYLAND_DISPLAY"])
        display = host_display if host_display.is_absolute() else host_runtime / host_display
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(Path(root) / "config"), WAYLAND_DISPLAY=str(display))
        with (Path(root) / "villain.log").open("w") as log:
            proc = subprocess.Popen([str(BINARY), "--winit"], env=env, stdout=log, stderr=log)
            sockets = []
            try:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    paths = list(runtime.glob("knave/desktop-*.sock"))
                    if paths:
                        break
                    if proc.poll() is not None:
                        raise RuntimeError((Path(root) / "villain.log").read_text())
                    time.sleep(0.05)
                else:
                    raise RuntimeError("nested compositor did not create desktop socket")
                address = str(paths[0])

                def connect():
                    sock = socket.socket(socket.AF_UNIX)
                    sock.settimeout(3)
                    sock.connect(address)
                    sockets.append(sock)
                    return sock, sock.makefile("rwb")

                sock, file = connect()
                subscribe = {"type": "subscribe", "payload": {"protocol": {"major": 1, "minor": 2}}}
                snapshot = {"type": "query", "payload": {"query": "snapshot"}}
                if mode == "idle-subscription" or mode == "stress":
                    initial = request(file, subscribe)
                    assert initial["type"] == "snapshot", initial
                else:
                    initial = request(file, snapshot)
                    assert initial["type"] == "snapshot", initial
                time.sleep(0.2)
                before = sample(proc.pid)
                if mode == "stress":
                    command_sock, command_file = connect()
                    times = []
                    generations = []
                    for index in range(40):
                        workspace = 2 + index % 2
                        start = time.perf_counter()
                        result = request(command_file, {"type": "dispatch", "payload": {"action": "focus-workspace", "workspace": workspace}})
                        update = json.loads(file.readline())
                        assert result["type"] == "ok" and update["type"] == "snapshot"
                        assert any(item["active"] and item["workspace"] == workspace for item in update["payload"]["workspaces"])
                        times.append((time.perf_counter() - start) * 1000)
                        generations.append(update["payload"]["generation"])
                    for _ in range(40):
                        churn_sock, churn_file = connect()
                        assert request(churn_file, subscribe)["type"] == "snapshot"
                        churn_file.close()
                        churn_sock.close()
                    time.sleep(0.2)
                    result = {"workspace_changes": 40, "connections_churned": 40,
                              "delivery_ms_median": statistics.median(times),
                              "delivery_ms_p95": sorted(times)[37], "delivery_ms_max": max(times),
                              "increasing_generations": all(a < b for a, b in zip(generations, generations[1:]))}
                    command_file.close()
                    command_sock.close()
                else:
                    start = time.monotonic()
                    count = 0
                    unsolicited = False
                    while time.monotonic() - start < DURATION:
                        if mode == "polling-baseline":
                            time.sleep(0.5)
                            assert request(file, snapshot)["type"] == "snapshot"
                            count += 1
                        else:
                            unsolicited |= bool(select.select([sock], [], [], min(0.5, DURATION - (time.monotonic() - start)))[0])
                    result = {"seconds": DURATION, "periodic_snapshot_queries": count,
                              "unsolicited_subscription_data": unsolicited}
                after = sample(proc.pid)
                print(json.dumps({"mode": mode, "before": before, "after": after,
                                  "delta_cpu_ticks": after["cpu_ticks"] - before["cpu_ticks"],
                                  "delta_context_switches": after["context_switches"] - before["context_switches"],
                                  **result}), flush=True)
            finally:
                for sock in sockets:
                    sock.close()
                if proc.poll() is None:
                    proc.terminate()
                proc.wait(timeout=5)


if __name__ == "__main__":
    for name in ("polling-baseline", "idle-subscription", "stress"):
        session(name)
