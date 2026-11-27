"""SciRes line protocol bridge to an IJulia/Jupyter kernel.

This process is supervised by the Rust JuliaKernel adapter. It translates a small
JSON-lines control protocol into Jupyter messaging and keeps the IJulia process
separate from the Rust IDE process.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from jupyter_client import KernelManager


def emit(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def collect_execution(client, message_id, timeout=30):
    outputs = []
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("kernel did not finish execution")
        message = client.get_iopub_msg(timeout=remaining)
        if message["parent_header"].get("msg_id") != message_id:
            continue
        message_type = message["msg_type"]
        if message_type == "status":
            if message["content"]["execution_state"] == "idle":
                break
        elif message_type == "stream":
            outputs.append({
                "kind": "stderr" if message["content"]["name"] == "stderr" else "stdout",
                "data": message["content"]["text"],
            })
        elif message_type == "execute_result":
            outputs.append({
                "kind": "display",
                "data": message["content"]["data"].get("text/plain", ""),
            })
        elif message_type == "display_data":
            outputs.append({
                "kind": "display",
                "data": message["content"]["data"].get("text/plain", ""),
            })
        elif message_type == "error":
            outputs.append({
                "kind": "stderr",
                "data": message["content"]["ename"] + ": " + message["content"]["evalue"],
            })

    shell_deadline = deadline
    while True:
        remaining = max(0.1, shell_deadline - time.monotonic())
        reply = client.get_shell_msg(timeout=remaining)
        if reply["parent_header"].get("msg_id") == message_id:
            return outputs, reply["content"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--julia", required=True)
    parser.add_argument("--project", required=True)
    args = parser.parse_args()

    project = str(Path(args.project).resolve()).replace("\\", "/")
    julia = args.julia
    with tempfile.TemporaryDirectory(prefix="scires-jupyter-data-") as data_dir:
        os.environ["JUPYTER_DATA_DIR"] = data_dir
        installer = (
            "using IJulia; "
            'println(IJulia.installkernel("SciRes Julia", '
            '"--startup-file=no", "--project=' + project.replace('"', '\\"') +
            '"; specname="scires-julia", displayname="SciRes Julia"))'
        )
        subprocess.run(
            [julia, "--project=" + project, "-e", installer],
            check=True,
            timeout=120,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )

        manager = None
        client = None

        def stop(now=False):
            nonlocal manager, client
            if client is not None:
                client.stop_channels()
            if manager is not None and manager.has_kernel:
                manager.shutdown_kernel(now=now)
            client = None
            manager = None

        def start():
            nonlocal manager, client
            manager = KernelManager(kernel_name="scires-julia")
            saved_stdout = os.dup(1)
            devnull = os.open(os.devnull, os.O_WRONLY)
            try:
                os.dup2(devnull, 1)
                manager.start_kernel()
                client = manager.client()
                client.start_channels()
                client.wait_for_ready(timeout=120)
            finally:
                os.dup2(saved_stdout, 1)
                os.close(saved_stdout)
                os.close(devnull)
            return True

        for raw_line in sys.stdin:
            try:
                command = json.loads(raw_line)
            except json.JSONDecodeError as error:
                emit({"ok": False, "error": "invalid JSON command: " + str(error)})
                continue

            action = command.get("command")
            try:
                if action == "start":
                    if manager is None:
                        start()
                    outputs, reply = collect_execution(
                        client,
                        client.execute('string(VERSION)'),
                        timeout=30,
                    )
                    version = "unknown"
                    if reply["status"] == "ok":
                        for output in outputs:
                            if output["kind"] == "display":
                                version = output["data"].strip('"')
                    emit({
                        "ok": True,
                        "info": {
                            "language": "julia",
                            "implementation": "IJulia",
                            "language_version": version,
                        },
                    })
                elif action == "execute":
                    if manager is None:
                        raise RuntimeError("kernel is not started")
                    message_id = client.execute(command["code"])
                    outputs, reply = collect_execution(client, message_id)
                    if reply["status"] == "error":
                        error_outputs = [
                            output for output in outputs
                            if output["kind"] == "stderr"
                        ]
                        error_text = error_outputs[-1]["data"] if error_outputs else "ExecutionError"
                        emit({
                            "ok": False,
                            "ename": "ExecutionError",
                            "evalue": error_text,
                            "outputs": outputs,
                        })
                    else:
                        emit({
                            "ok": True,
                            "execution_count": reply.get("execution_count", 1),
                            "outputs": outputs,
                        })
                elif action == "interrupt":
                    if manager is None:
                        raise RuntimeError("kernel is not started")
                    manager.interrupt_kernel()
                    emit({"ok": True})
                elif action == "restart":
                    stop(now=False)
                    start()
                    emit({"ok": True})
                elif action == "shutdown":
                    stop(now=False)
                    emit({"ok": True})
                    break
                else:
                    emit({"ok": False, "error": "unknown command: " + str(action)})
            except Exception as error:
                emit({"ok": False, "error": str(error)})


if __name__ == "__main__":
    main()
