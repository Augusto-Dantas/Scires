//! Minimal IJulia/Jupyter reference-kernel adapter.
//!
//! The adapter owns a small Python Jupyter bridge process. The bridge owns the
//! IJulia kernel process and translates Jupyter execute/control messages into the
//! SciRes `Kernel` port. It does not make Julia a shared-memory or in-process runtime.
//!
//! # Examples
//!
//! ```
//! use scires_julia_kernel::{JuliaKernel, JuliaKernelConfig};
//! use scires_kernel::Kernel;
//! use std::path::PathBuf;
//!
//! let kernel = JuliaKernel::new(JuliaKernelConfig {
//!     julia: PathBuf::from("julia"),
//!     project: PathBuf::from("project"),
//!     python: PathBuf::from("python"),
//!     bridge_script: PathBuf::from("julia_bridge.py"),
//! });
//! assert_eq!(kernel.handshake().version, 1);
//! ```

use scires_core::{Language, Variable};
use scires_kernel::{
    ExecuteRequest, ExecuteResult, Kernel, KernelCapabilities, KernelError, KernelHandshake,
    KernelInfo, KernelStatus, Output,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

/// Configuration for the Julia reference kernel.
#[derive(Debug, Clone)]
pub struct JuliaKernelConfig {
    /// Julia executable.
    pub julia: PathBuf,
    /// Julia project environment.
    pub project: PathBuf,
    /// Python executable containing `jupyter_client` and `pyzmq`.
    pub python: PathBuf,
    /// Bridge script shipped by this adapter crate.
    pub bridge_script: PathBuf,
}

impl JuliaKernelConfig {
    /// Resolve the workspace test bridge and environment.
    pub fn test_default(root: &Path) -> Option<Self> {
        let julia = which("julia")?;
        let python = std::env::var_os("SCIRES_JULIA_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("target")
                    .join("julia-test-venv")
                    .join("Scripts")
                    .join("python.exe")
            });
        let python = if python.is_file() {
            python
        } else {
            which("python")?
        };
        Some(Self {
            julia,
            project: root.join("tests").join("julia"),
            python,
            bridge_script: root
                .join("crates")
                .join("scires-julia-kernel")
                .join("julia_bridge.py"),
        })
    }
}

/// Factory for the Julia reference kernel.
#[derive(Debug, Clone)]
pub struct JuliaKernelFactory {
    config: JuliaKernelConfig,
}

impl JuliaKernelFactory {
    /// Create a factory with explicit runtime configuration.
    pub fn new(config: JuliaKernelConfig) -> Self {
        Self { config }
    }
}

impl scires_kernel::KernelFactory for JuliaKernelFactory {
    fn language(&self) -> Language {
        Language::Julia
    }

    fn create(&self) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(JuliaKernel::new(self.config.clone())))
    }
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            });
            candidate.is_file().then_some(candidate)
        })
    })
}

/// Long-lived Julia kernel supervised through a Jupyter-compatible bridge.
pub struct JuliaKernel {
    config: JuliaKernelConfig,
    status: KernelStatus,
    child: Option<Child>,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: Option<BufReader<std::process::ChildStdout>>,
    handshake: Option<KernelHandshake>,
    count: u64,
}

impl JuliaKernel {
    /// Create a not-yet-started Julia kernel.
    pub fn new(config: JuliaKernelConfig) -> Self {
        Self {
            config,
            status: KernelStatus::NotStarted,
            child: None,
            stdin: None,
            stdout: None,
            handshake: None,
            count: 0,
        }
    }

    fn send(&mut self, command: Value) -> Result<Value, KernelError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| KernelError::Startup("bridge is not started".into()))?;
        serde_json::to_writer(&mut *stdin, &command).map_err(|error| {
            KernelError::Startup(format!("failed to encode bridge command: {error}"))
        })?;
        stdin
            .write_all(b"\n")
            .and_then(|_| stdin.flush())
            .map_err(|error| {
                KernelError::Startup(format!("failed to write bridge command: {error}"))
            })?;

        let stdout = self
            .stdout
            .as_mut()
            .ok_or_else(|| KernelError::Startup("bridge has no stdout".into()))?;
        let mut line = String::new();
        let read = stdout.read_line(&mut line).map_err(|error| {
            KernelError::Startup(format!("failed to read bridge reply: {error}"))
        })?;
        if read == 0 {
            return Err(KernelError::Startup("bridge exited without a reply".into()));
        }
        serde_json::from_str(&line).map_err(|error| {
            KernelError::Startup(format!("bridge returned invalid JSON: {error}; {line}"))
        })
    }
}

impl Kernel for JuliaKernel {
    fn info(&self) -> KernelInfo {
        self.handshake
            .as_ref()
            .map(|handshake| handshake.info.clone())
            .unwrap_or_else(|| KernelInfo {
                language: Language::Julia,
                implementation: "IJulia".into(),
                language_version: "unknown".into(),
            })
    }

    fn status(&self) -> KernelStatus {
        self.status
    }

    fn start(&mut self) -> Result<(), KernelError> {
        if !self.config.bridge_script.is_file() {
            return Err(KernelError::Startup(format!(
                "bridge script not found: {}",
                self.config.bridge_script.display()
            )));
        }
        let mut child = Command::new(&self.config.python)
            .arg(&self.config.bridge_script)
            .arg("--julia")
            .arg(&self.config.julia)
            .arg("--project")
            .arg(&self.config.project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                KernelError::Startup(format!(
                    "failed to start Jupyter bridge {}: {error}",
                    self.config.python.display()
                ))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| KernelError::Startup("Jupyter bridge stdin is unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| KernelError::Startup("Jupyter bridge stdout is unavailable".into()))?;
        self.stdin = Some(BufWriter::new(stdin));
        self.stdout = Some(BufReader::new(stdout));
        self.child = Some(child);

        let reply = self.send(json!({"command": "start"}))?;
        if reply["ok"] != true {
            return Err(KernelError::Startup(
                reply["error"]
                    .as_str()
                    .unwrap_or("unknown startup failure")
                    .into(),
            ));
        }
        let info: KernelInfo = serde_json::from_value(reply["info"].clone()).map_err(|error| {
            KernelError::Startup(format!("invalid kernel info from bridge: {error}"))
        })?;
        self.handshake = Some(KernelHandshake::new(
            info,
            KernelCapabilities {
                execute: true,
                interrupt: false,
                shutdown: true,
                restart: true,
            },
        ));
        self.status = KernelStatus::Idle;
        Ok(())
    }

    fn execute(&mut self, req: &ExecuteRequest) -> Result<ExecuteResult, KernelError> {
        if self.status != KernelStatus::Idle {
            return Err(KernelError::InvalidStatus(self.status));
        }
        self.status = KernelStatus::Busy;
        let reply = self.send(json!({"command": "execute", "code": req.code}));
        self.status = KernelStatus::Idle;
        let reply = reply?;
        if reply["ok"] != true {
            return Err(KernelError::Execution {
                ename: reply["ename"].as_str().unwrap_or("ExecutionError").into(),
                evalue: reply["evalue"]
                    .as_str()
                    .unwrap_or("execution failed")
                    .into(),
            });
        }
        #[derive(Deserialize)]
        struct BridgeOutput {
            kind: String,
            data: String,
        }
        let outputs: Vec<BridgeOutput> =
            serde_json::from_value(reply["outputs"].clone()).map_err(|error| {
                KernelError::Execution {
                    ename: "ProtocolError".into(),
                    evalue: format!("invalid bridge outputs: {error}"),
                }
            })?;
        self.count = reply["execution_count"].as_u64().unwrap_or(self.count + 1);
        Ok(ExecuteResult {
            execution_count: self.count,
            outputs: outputs
                .into_iter()
                .map(|output| match output.kind.as_str() {
                    "stderr" => Output::Stderr(output.data),
                    "display" => Output::Display {
                        mime: "text/plain".into(),
                        data: output.data,
                    },
                    _ => Output::Stdout(output.data),
                })
                .collect(),
        })
    }

    fn interrupt(&mut self) -> Result<(), KernelError> {
        Err(KernelError::CapabilityUnavailable("interrupt"))
    }

    fn variables(&mut self) -> Result<Vec<Variable>, KernelError> {
        Ok(Vec::new())
    }

    fn shutdown(&mut self) -> Result<(), KernelError> {
        if let (Some(_), true) = (&mut self.stdin, self.child.is_some()) {
            let reply = self.send(json!({"command": "shutdown"}));
            match reply {
                Ok(reply) if reply["ok"] == true => {}
                Ok(reply) => {
                    return Err(KernelError::Startup(
                        reply["error"].as_str().unwrap_or("shutdown failed").into(),
                    ))
                }
                Err(error) => return Err(error),
            }
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
        self.stdin = None;
        self.stdout = None;
        self.status = KernelStatus::Dead;
        Ok(())
    }

    fn restart(&mut self) -> Result<(), KernelError> {
        if matches!(self.status, KernelStatus::NotStarted | KernelStatus::Dead) {
            return self.start();
        }
        let reply = self.send(json!({"command": "restart"}))?;
        if reply["ok"] != true {
            return Err(KernelError::Startup(
                reply["error"].as_str().unwrap_or("restart failed").into(),
            ));
        }
        self.count = 0;
        self.status = KernelStatus::Idle;
        Ok(())
    }

    fn handshake(&self) -> KernelHandshake {
        self.handshake.clone().unwrap_or_else(|| {
            KernelHandshake::new(
                KernelInfo {
                    language: Language::Julia,
                    implementation: "IJulia".into(),
                    language_version: "unknown".into(),
                },
                KernelCapabilities {
                    execute: true,
                    interrupt: false,
                    shutdown: true,
                    restart: true,
                },
            )
        })
    }
}

impl Drop for JuliaKernel {
    fn drop(&mut self) {
        if self.status != KernelStatus::NotStarted {
            if let Ok(reply) = self.send(json!({"command": "shutdown"})) {
                if reply["ok"] == true {
                    if let Some(mut child) = self.child.take() {
                        let _ = child.wait();
                    }
                }
            }
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scires_kernel::KernelFactory;

    fn integration_config() -> Option<JuliaKernelConfig> {
        let python = std::env::var_os("SCIRES_JULIA_PYTHON").map(PathBuf::from)?;
        if !python.is_file() {
            return None;
        }
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .map(ToOwned::to_owned)?;
        let julia = which("julia")?;
        Some(JuliaKernelConfig {
            julia,
            project: root.join("tests").join("julia"),
            python,
            bridge_script: root
                .join("crates")
                .join("scires-julia-kernel")
                .join("julia_bridge.py"),
        })
    }

    #[test]
    fn julia_factory_targets_julia() {
        let config = JuliaKernelConfig {
            julia: PathBuf::from("julia"),
            project: PathBuf::from("tests/julia"),
            python: PathBuf::from("python"),
            bridge_script: PathBuf::from("crates/scires-julia-kernel/julia_bridge.py"),
        };
        let factory = JuliaKernelFactory::new(config);
        assert_eq!(factory.language(), Language::Julia);
    }

    #[test]
    fn julia_kernel_reports_versioned_handshake() {
        let config = JuliaKernelConfig {
            julia: PathBuf::from("julia"),
            project: PathBuf::from("tests/julia"),
            python: PathBuf::from("python"),
            bridge_script: PathBuf::from("crates/scires-julia-kernel/julia_bridge.py"),
        };
        let kernel = JuliaKernel::new(config);
        let handshake = kernel.handshake();
        assert_eq!(handshake.api, "scires.kernel");
        assert_eq!(handshake.version, 1);
        assert_eq!(handshake.info.language, Language::Julia);
        assert!(handshake.capabilities.execute);
        assert!(!handshake.capabilities.interrupt);
        assert!(handshake.capabilities.shutdown);
        assert!(handshake.capabilities.restart);
    }

    #[test]
    #[ignore = "requires SCIRES_JULIA_PYTHON and the pinned Julia test environment"]
    fn julia_bridge_round_trip_is_available_when_environment_is_configured() {
        let Some(config) = integration_config() else {
            return;
        };
        let mut kernel = JuliaKernel::new(config);
        kernel.start().expect("Julia bridge should start");
        assert_eq!(kernel.status(), KernelStatus::Idle);
        assert_eq!(kernel.handshake().version, 1);

        let result = kernel
            .execute(&ExecuteRequest::new("println(\"rust-bridge-ok\")"))
            .expect("Julia execution should succeed");
        assert!(result.outputs.iter().any(|output| {
            matches!(output, Output::Stdout(text) if text.contains("rust-bridge-ok"))
        }));

        let error = kernel
            .execute(&ExecuteRequest::new("error(\"rust-bridge-error\")"))
            .expect_err("Julia execution should return an execution error");
        assert!(matches!(
            error,
            KernelError::Execution { ename, evalue }
                if ename == "ExecutionError"
                    && evalue.contains("rust-bridge-error")
        ));

        kernel.restart().expect("Julia restart should succeed");
        let result = kernel
            .execute(&ExecuteRequest::new(
                "isdefined(Main, :never_defined_in_restart)",
            ))
            .expect("Julia execution after restart should succeed");
        assert!(result.execution_count >= 1);

        kernel.shutdown().expect("Julia shutdown should succeed");
        assert_eq!(kernel.status(), KernelStatus::Dead);
    }
}
