"""Exercise IJulia lifecycle through the Jupyter client protocol.

Run with: python tests/julia_kernel_lifecycle.py
Install the pinned client dependency from tests/requirements-julia-kernel.txt first.
"""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

from jupyter_client import KernelManager


ROOT = Path(__file__).resolve().parents[1]
JULIA_PROJECT = ROOT / "tests" / "julia"
JULIA_INSTALLER = JULIA_PROJECT / "install_kernel.jl"


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
        else:
            outputs.append(message)

    while True:
        reply = client.get_shell_msg(timeout=max(0.1, deadline - time.monotonic()))
        if reply["parent_header"].get("msg_id") == message_id:
            return outputs, reply["content"]


def execute(client, code, timeout=30):
    return collect_execution(client, client.execute(code), timeout)


def start_kernel():
    manager = KernelManager(kernel_name="scires-julia")
    manager.start_kernel()
    client = manager.client()
    client.start_channels()
    client.wait_for_ready(timeout=120)
    return manager, client


def stop_kernel(manager, client):
    client.stop_channels()
    if manager.has_kernel:
        manager.shutdown_kernel(now=False)


def main():
    julia = shutil.which("julia")
    if julia is None:
        raise RuntimeError("Julia executable was not found on PATH")

    with tempfile.TemporaryDirectory(prefix="scires-jupyter-data-") as data_dir:
        env = os.environ.copy()
        env["JUPYTER_DATA_DIR"] = data_dir
        os.environ["JUPYTER_DATA_DIR"] = data_dir
        subprocess.run(
            [
                julia,
                "--project=" + str(JULIA_PROJECT),
                str(JULIA_INSTALLER),
            ],
            cwd=ROOT,
            env=env,
            check=True,
            timeout=120,
        )

        manager, client = start_kernel()
        try:
            outputs, reply = execute(
                client,
                'scires_restart_probe = 42\nprintln("stdout-check")\n'
                'Base.active_project()',
            )
            assert reply["status"] == "ok", reply
            stream = next(item for item in outputs if item["msg_type"] == "stream")
            result = next(
                item for item in outputs if item["msg_type"] == "execute_result"
            )
            assert stream["content"]["name"] == "stdout"
            assert stream["content"]["text"] == "stdout-check\n"
            active_project = (
                result["content"]["data"]["text/plain"]
                .strip('"')
                .replace("\\", "/")
                .replace("//", "/")
            )
            assert active_project.endswith("/tests/julia/Project.toml"), active_project

            error_outputs, error_reply = execute(client, 'error("intentional-test-error")')
            error = next(item for item in error_outputs if item["msg_type"] == "error")
            assert error_reply["status"] == "error"
            assert "intentional-test-error" in error["content"]["evalue"]

            interrupted_id = client.execute("sleep(60)")
            while True:
                message = client.get_iopub_msg(timeout=30)
                if message["parent_header"].get("msg_id") == interrupted_id:
                    if (
                        message["msg_type"] == "status"
                        and message["content"]["execution_state"] == "busy"
                    ):
                        break
            manager.interrupt_kernel()
            interrupted_outputs, interrupted_reply = collect_execution(
                client, interrupted_id, timeout=30
            )
            assert interrupted_reply["status"] == "error"
            assert any(item["msg_type"] == "error" for item in interrupted_outputs)
        finally:
            stop_kernel(manager, client)

        manager, client = start_kernel()
        try:
            outputs, reply = execute(client, "isdefined(Main, :scires_restart_probe)")
            result = next(
                item for item in outputs if item["msg_type"] == "execute_result"
            )
            assert reply["status"] == "ok"
            assert result["content"]["data"]["text/plain"] == "false"
        finally:
            stop_kernel(manager, client)

    print("IJulia/Jupyter launch, environment, execution, error, interrupt, shutdown and restart passed")


if __name__ == "__main__":
    main()
