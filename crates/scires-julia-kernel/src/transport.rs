//! Direct pure-Rust ZeroMQ + Tokio transport for an out-of-process IJulia kernel.

use crate::protocol::{ConnectionInfo, ExecutionAccumulator, JupyterMessage, TempConnectionFile};
use crate::JuliaKernelConfig;
use bytes::Bytes;
use scires_core::Language;
use scires_kernel::{
    ExecuteRequest, ExecuteResult, KernelCapabilities, KernelControl, KernelError,
    KernelHandshake, KernelInfo,
};
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;
use zeromq::{DealerSocket, Socket, SocketRecv, SocketSend, SubSocket, ZmqMessage};

/// Thread-safe control handle for interrupting an active IJulia execution over
/// the Jupyter `control` channel, with signal/kill escalation if the kernel hangs.
pub(crate) struct JuliaKernelControl {
    runtime: Arc<tokio::runtime::Runtime>,
    control: Arc<tokio::sync::Mutex<DealerSocket>>,
    key: String,
    session_id: String,
    child: Arc<Mutex<Option<Child>>>,
    interrupt_grace: Duration,
}

impl KernelControl for JuliaKernelControl {
    fn interrupt(&self) -> Result<(), KernelError> {
        // On Unix, IJulia also listens for SIGINT (its default interrupt_mode = "signal"),
        // while newer IJulia handles `interrupt_request` on the control channel.
        // Send `interrupt_request` on `control` first; if no reply arrives within the
        // first half of `interrupt_grace`, deliver SIGINT on Unix; if still unresponsive,
        // escalate to terminating the child process.
        let msg_id = Uuid::new_v4().to_string();
        let req =
            JupyterMessage::new_request(&self.session_id, &msg_id, "interrupt_request", json!({}));
        let zmq_msg = to_zmq_message(&req, &self.key)?;

        let half_grace = (self.interrupt_grace / 2).max(Duration::from_millis(250));
        let child = Arc::clone(&self.child);

        let reply_result = self.runtime.block_on(async {
            let mut socket = self.control.lock().await;
            socket.send(zmq_msg).await.map_err(|error| {
                KernelError::Transport(format!("failed to send interrupt_request: {error}"))
            })?;

            let recv_fut = recv_matching_reply(&mut socket, &self.key, &msg_id);
            tokio::pin!(recv_fut);

            tokio::select! {
                res = &mut recv_fut => res.map(|_| ()),
                _ = tokio::time::sleep(half_grace) => {
                    // Fallback: deliver SIGINT on Unix so a busy Julia task raises InterruptException.
                    send_sigint(&child);
                    tokio::select! {
                        res = &mut recv_fut => res.map(|_| ()),
                        _ = tokio::time::sleep(self.interrupt_grace.saturating_sub(half_grace)) => {
                            Err(KernelError::Timeout(
                                "kernel did not acknowledge interrupt_request before grace deadline"
                                    .into(),
                            ))
                        }
                    }
                }
            }
        });

        if let Err(err) = reply_result {
            if matches!(err, KernelError::Timeout(_)) {
                kill_child(&self.child);
            }
            return Err(err);
        }
        Ok(())
    }
}

/// Active direct ZeroMQ session to a supervised IJulia process.
pub(crate) struct JuliaSession {
    runtime: Arc<tokio::runtime::Runtime>,
    _conn_file: TempConnectionFile,
    key: String,
    session_id: String,
    child: Arc<Mutex<Option<Child>>>,
    shell: DealerSocket,
    iopub_rx: mpsc::UnboundedReceiver<Result<JupyterMessage, KernelError>>,
    _iopub_task: tokio::task::JoinHandle<()>,
    control: Arc<tokio::sync::Mutex<DealerSocket>>,
    handshake: KernelHandshake,
    execute_timeout: Duration,
    interrupt_grace: Duration,
    shutdown_timeout: Duration,
}

impl JuliaSession {
    /// Launch Julia with IJulia, connect ZeroMQ channels, and complete `kernel_info` handshake.
    pub(crate) fn launch(config: &JuliaKernelConfig) -> Result<Self, KernelError> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|error| {
                    KernelError::Startup(format!("failed to initialize async runtime: {error}"))
                })?,
        );

        let conn = ConnectionInfo::allocate_ephemeral()?;
        let conn_file = TempConnectionFile::create(&conn)?;
        let session_id = Uuid::new_v4().to_string();

        let child_proc = Command::new(&config.julia)
            .arg("--color=yes")
            .arg("--startup-file=no")
            .arg(format!("--project={}", config.project.display()))
            .arg("-e")
            .arg("using IJulia; IJulia.run_kernel()")
            .arg(conn_file.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                KernelError::Startup(format!(
                    "failed to spawn Julia executable {}: {error}",
                    config.julia.display()
                ))
            })?;

        let child = Arc::new(Mutex::new(Some(child_proc)));

        let connect_result = runtime.block_on(async {
            let mut shell = DealerSocket::new();
            shell
                .connect(&conn.endpoint(conn.shell_port))
                .await
                .map_err(|error| {
                    KernelError::Startup(format!("failed to connect shell socket: {error}"))
                })?;

            let mut iopub = SubSocket::new();
            iopub
                .connect(&conn.endpoint(conn.iopub_port))
                .await
                .map_err(|error| {
                    KernelError::Startup(format!("failed to connect iopub socket: {error}"))
                })?;
            iopub.subscribe("").await.map_err(|error| {
                KernelError::Startup(format!("failed to subscribe to iopub socket: {error}"))
            })?;

            let mut control = DealerSocket::new();
            control
                .connect(&conn.endpoint(conn.control_port))
                .await
                .map_err(|error| {
                    KernelError::Startup(format!("failed to connect control socket: {error}"))
                })?;

            let (iopub_tx, iopub_rx) = mpsc::unbounded_channel();
            let iopub_key = conn.key.clone();
            let iopub_task = tokio::spawn(async move {
                loop {
                    match iopub.recv().await {
                        Ok(raw) => {
                            let decoded = from_zmq_message(&raw, &iopub_key);
                            if iopub_tx.send(decoded).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = iopub_tx.send(Err(KernelError::Transport(format!(
                                "iopub receive failed: {error}"
                            ))));
                            break;
                        }
                    }
                }
            });

            let info = handshake_kernel_info(
                &mut shell,
                &conn.key,
                &session_id,
                &child,
                config.startup_timeout,
            )
            .await?;

            Ok::<_, KernelError>((shell, iopub_rx, iopub_task, control, info))
        });

        let (shell, iopub_rx, iopub_task, control, info) = match connect_result {
            Ok(parts) => parts,
            Err(error) => {
                kill_child(&child);
                return Err(error);
            }
        };

        let handshake = KernelHandshake::new(
            info,
            KernelCapabilities {
                execute: true,
                interrupt: true,
                shutdown: true,
                restart: true,
            },
        );

        Ok(Self {
            runtime,
            _conn_file: conn_file,
            key: conn.key,
            session_id,
            child,
            shell,
            iopub_rx,
            _iopub_task: iopub_task,
            control: Arc::new(tokio::sync::Mutex::new(control)),
            handshake,
            execute_timeout: config.execute_timeout,
            interrupt_grace: config.interrupt_grace,
            shutdown_timeout: config.shutdown_timeout,
        })
    }

    pub(crate) fn handshake(&self) -> &KernelHandshake {
        &self.handshake
    }

    pub(crate) fn control_handle(&self) -> Arc<dyn KernelControl> {
        Arc::new(JuliaKernelControl {
            runtime: Arc::clone(&self.runtime),
            control: Arc::clone(&self.control),
            key: self.key.clone(),
            session_id: self.session_id.clone(),
            child: Arc::clone(&self.child),
            interrupt_grace: self.interrupt_grace,
        })
    }

    /// Check whether the supervised Julia process has exited unexpectedly.
    pub(crate) fn check_alive(&self) -> Result<(), KernelError> {
        check_child_alive(&self.child)
    }

    /// Execute `req` on the shell channel and correlate all IOPub messages and reply.
    pub(crate) fn execute(&mut self, req: &ExecuteRequest) -> Result<ExecuteResult, KernelError> {
        self.check_alive()?;
        let msg = JupyterMessage::new_request(
            &self.session_id,
            req.request_id.as_str(),
            "execute_request",
            json!({
                "code": req.code,
                "silent": req.silent,
                "store_history": !req.silent,
                "user_expressions": {},
                "allow_stdin": false,
                "stop_on_error": true,
            }),
        );
        let zmq_msg = to_zmq_message(&msg, &self.key)?;

        let key = self.key.clone();
        let child = Arc::clone(&self.child);
        let execute_timeout = self.execute_timeout;
        let req_id = req.request_id.clone();

        let shell = &mut self.shell;
        let iopub_rx = &mut self.iopub_rx;

        self.runtime.block_on(async move {
            shell.send(zmq_msg).await.map_err(|error| {
                KernelError::Transport(format!("failed to send execute_request: {error}"))
            })?;

            let mut acc = ExecutionAccumulator::new(req_id.clone());
            let mut shell_reply: Option<JupyterMessage> = None;
            let deadline = tokio::time::Instant::now() + execute_timeout;
            let mut liveness = tokio::time::interval(Duration::from_millis(250));
            liveness.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            // Pin the shell receiver future so periodic liveness checks and IOPub
            // deliveries never cancel an in-flight ZeroMQ receive.
            let shell_fut = recv_matching_reply(shell, &key, req_id.as_str());
            tokio::pin!(shell_fut);

            while !(acc.is_idle() && shell_reply.is_some()) {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(KernelError::Timeout(format!(
                        "execution {} exceeded deadline of {:?} (idle={}, shell_reply={})",
                        req_id,
                        execute_timeout,
                        acc.is_idle(),
                        shell_reply.is_some()
                    )));
                }

                tokio::select! {
                    msg = iopub_rx.recv(), if !acc.is_idle() => {
                        let decoded = msg.ok_or_else(|| {
                            KernelError::Transport("iopub channel closed unexpectedly".into())
                        })??;
                        acc.handle_iopub(&decoded);
                    }
                    res = &mut shell_fut, if shell_reply.is_none() => {
                        shell_reply = Some(res?);
                    }
                    _ = liveness.tick() => {
                        check_child_alive(&child)?;
                    }
                    _ = tokio::time::sleep(remaining) => {
                        return Err(KernelError::Timeout(format!(
                            "execution {} exceeded deadline of {:?} (idle={}, shell_reply={})",
                            req_id,
                            execute_timeout,
                            acc.is_idle(),
                            shell_reply.is_some()
                        )));
                    }
                }
            }

            let reply = shell_reply.ok_or_else(|| {
                KernelError::Protocol("missing shell execute_reply after idle".into())
            })?;
            acc.finish(&reply)
        })
    }

    /// Gracefully shut down the Julia kernel via `shutdown_request`, escalating to kill if needed.
    pub(crate) fn shutdown(&mut self) -> Result<(), KernelError> {
        let msg_id = Uuid::new_v4().to_string();
        let req = JupyterMessage::new_request(
            &self.session_id,
            &msg_id,
            "shutdown_request",
            json!({ "restart": false }),
        );
        if let Ok(zmq_msg) = to_zmq_message(&req, &self.key) {
            let control = Arc::clone(&self.control);
            let key = self.key.clone();
            let timeout = self.shutdown_timeout;
            let _ = self.runtime.block_on(async move {
                let mut socket = control.lock().await;
                if socket.send(zmq_msg).await.is_ok() {
                    let _ = tokio::time::timeout(
                        timeout,
                        recv_matching_reply(&mut socket, &key, &msg_id),
                    )
                    .await;
                }
            });
        }

        wait_or_kill_child(&self.child, self.shutdown_timeout);
        Ok(())
    }
}

impl Drop for JuliaSession {
    fn drop(&mut self) {
        self._iopub_task.abort();
        kill_child(&self.child);
    }
}

async fn recv_matching_reply(
    socket: &mut DealerSocket,
    key: &str,
    target_msg_id: &str,
) -> Result<JupyterMessage, KernelError> {
    loop {
        let raw = socket.recv().await.map_err(|error| {
            KernelError::Transport(format!("ZeroMQ receive failed: {error}"))
        })?;
        let decoded = from_zmq_message(&raw, key)?;
        if decoded.parent_msg_id() == Some(target_msg_id) {
            return Ok(decoded);
        }
    }
}

async fn handshake_kernel_info(
    shell: &mut DealerSocket,
    key: &str,
    session_id: &str,
    child: &Arc<Mutex<Option<Child>>>,
    timeout: Duration,
) -> Result<KernelInfo, KernelError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let msg_id = Uuid::new_v4().to_string();
    let req = JupyterMessage::new_request(session_id, &msg_id, "kernel_info_request", json!({}));
    let zmq_msg = to_zmq_message(&req, key)?;
    shell.send(zmq_msg).await.map_err(|error| {
        KernelError::Startup(format!("failed to send kernel_info_request: {error}"))
    })?;

    let recv_fut = recv_matching_reply(shell, key, &msg_id);
    tokio::pin!(recv_fut);

    let mut liveness = tokio::time::interval(Duration::from_millis(250));
    liveness.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(KernelError::Timeout(format!(
                "IJulia did not reply to kernel_info_request within {:?}",
                timeout
            )));
        }

        tokio::select! {
            res = &mut recv_fut => {
                let reply = res.map_err(|err| KernelError::Startup(err.to_string()))?;
                let implementation = reply
                    .content
                    .get("implementation")
                    .and_then(Value::as_str)
                    .unwrap_or("IJulia")
                    .to_owned();
                let language_version = reply
                    .content
                    .get("language_info")
                    .and_then(|info| info.get("version"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                return Ok(KernelInfo {
                    language: Language::Julia,
                    implementation,
                    language_version,
                });
            }
            _ = liveness.tick() => {
                check_child_alive(child).map_err(|err| KernelError::Startup(err.to_string()))?;
            }
            _ = tokio::time::sleep(remaining) => {
                return Err(KernelError::Timeout(format!(
                    "IJulia did not reply to kernel_info_request within {:?}",
                    timeout
                )));
            }
        }
    }
}

fn to_zmq_message(msg: &JupyterMessage, key: &str) -> Result<ZmqMessage, KernelError> {
    let frames = msg.encode(key)?;
    let mut iter = frames.into_iter();
    let first = iter
        .next()
        .ok_or_else(|| KernelError::Protocol("encoded Jupyter message has no frames".into()))?;
    let mut zmq_msg = ZmqMessage::from(first);
    for frame in iter {
        zmq_msg.push_back(Bytes::from(frame));
    }
    Ok(zmq_msg)
}

fn from_zmq_message(raw: &ZmqMessage, key: &str) -> Result<JupyterMessage, KernelError> {
    let slices: Vec<&[u8]> = raw.iter().map(Bytes::as_ref).collect();
    JupyterMessage::decode(&slices, key)
}

fn send_sigint(child: &Arc<Mutex<Option<Child>>>) {
    #[cfg(unix)]
    {
        if let Ok(guard) = child.lock() {
            if let Some(proc) = guard.as_ref() {
                let _ = Command::new("kill")
                    .arg("-INT")
                    .arg(proc.id().to_string())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child;
    }
}

fn check_child_alive(child: &Arc<Mutex<Option<Child>>>) -> Result<(), KernelError> {
    let mut guard = child
        .lock()
        .map_err(|_| KernelError::Transport("kernel process lock poisoned".into()))?;
    if let Some(proc) = guard.as_mut() {
        if let Some(status) = proc.try_wait().map_err(|error| {
            KernelError::Transport(format!("failed to poll Julia process status: {error}"))
        })? {
            *guard = None;
            return Err(KernelError::Transport(format!(
                "Julia kernel process exited unexpectedly with status {status}"
            )));
        }
        Ok(())
    } else {
        Err(KernelError::Transport(
            "Julia kernel process is no longer running".into(),
        ))
    }
}

fn wait_or_kill_child(child: &Arc<Mutex<Option<Child>>>, grace: Duration) {
    let deadline = Instant::now() + grace;
    if let Ok(mut guard) = child.lock() {
        if let Some(mut proc) = guard.take() {
            loop {
                match proc.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    _ => {
                        let _ = proc.kill();
                        let _ = proc.wait();
                        return;
                    }
                }
            }
        }
    }
}

fn kill_child(child: &Arc<Mutex<Option<Child>>>) {
    if let Ok(mut guard) = child.lock() {
        if let Some(mut proc) = guard.take() {
            let _ = proc.kill();
            let _ = proc.wait();
        }
    }
}
