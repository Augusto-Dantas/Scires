//! Direct pure-Rust Jupyter/ZeroMQ adapter for an out-of-process IJulia kernel.
//!
//! Per ADR-0010, this adapter spawns Julia directly with an ephemeral loopback
//! Jupyter connection file (`127.0.0.1`, HMAC-SHA256) and communicates over
//! pure-Rust ZeroMQ (`zeromq` + `tokio`) with no Python runtime dependency.
//!
//! # Examples
//!
//! ```
//! use scires_julia_kernel::{JuliaKernel, JuliaKernelConfig};
//! use scires_kernel::Kernel;
//!
//! let kernel = JuliaKernel::new(JuliaKernelConfig::new("julia", "tests/julia"));
//! let handshake = kernel.handshake();
//! assert_eq!(handshake.version, 1);
//! assert!(handshake.capabilities.interrupt);
//! ```

mod dataplane;
mod protocol;
mod transport;

use scires_core::{Language, Variable};
use scires_kernel::{
    ExecuteRequest, ExecuteResult, Kernel, KernelCapabilities, KernelControl, KernelError,
    KernelHandshake, KernelInfo, KernelStatus, ValueProfile,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use transport::JuliaSession;

const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_EXECUTE_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_INTERRUPT_GRACE: Duration = Duration::from_secs(5);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Configuration for the direct Rust Julia kernel adapter.
#[derive(Debug, Clone)]
pub struct JuliaKernelConfig {
    /// Path to the `julia` executable.
    pub julia: PathBuf,
    /// Path to the Julia project environment (`--project=...`).
    pub project: PathBuf,
    /// Maximum time to wait for IJulia startup and `kernel_info_reply`.
    pub startup_timeout: Duration,
    /// Maximum time to wait for a single `execute_request` to complete.
    pub execute_timeout: Duration,
    /// Maximum time to wait for `interrupt_reply` before escalating to termination.
    pub interrupt_grace: Duration,
    /// Maximum time to wait for graceful shutdown before killing the child process.
    pub shutdown_timeout: Duration,
}

impl JuliaKernelConfig {
    /// Create a configuration with explicit `julia` and `project` paths and default timeouts.
    pub fn new(julia: impl Into<PathBuf>, project: impl Into<PathBuf>) -> Self {
        Self {
            julia: julia.into(),
            project: project.into(),
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            execute_timeout: DEFAULT_EXECUTE_TIMEOUT,
            interrupt_grace: DEFAULT_INTERRUPT_GRACE,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
        }
    }

    /// Discover a `julia` executable explicitly via `SCIRES_JULIA_EXE`, `PATH`,
    /// or standard platform fallback paths (e.g. `/snap/bin/julia` on Linux).
    pub fn discover(project: impl Into<PathBuf>) -> Option<Self> {
        let julia = discover_julia()?;
        Some(Self::new(julia, project))
    }
}

/// Discover the `julia` executable in deterministic order:
/// 1. `SCIRES_JULIA_EXE` environment variable
/// 2. `PATH`
/// 3. `/snap/bin/julia` on Linux
pub fn discover_julia() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("SCIRES_JULIA_EXE") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(found) = which("julia") {
        return Some(found);
    }
    #[cfg(target_os = "linux")]
    {
        let snap = Path::new("/snap/bin/julia");
        if snap.is_file() {
            return Some(snap.to_path_buf());
        }
    }
    None
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

/// Factory for the direct Rust Julia kernel adapter.
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

/// Long-lived out-of-process Julia kernel supervised directly over ZeroMQ.
pub struct JuliaKernel {
    config: JuliaKernelConfig,
    status: KernelStatus,
    session: Option<JuliaSession>,
    handshake: Option<KernelHandshake>,
}

impl JuliaKernel {
    /// Create a not-yet-started Julia kernel.
    pub fn new(config: JuliaKernelConfig) -> Self {
        Self {
            config,
            status: KernelStatus::NotStarted,
            session: None,
            handshake: None,
        }
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
        self.status = KernelStatus::Starting;
        match JuliaSession::launch(&self.config) {
            Ok(session) => {
                self.handshake = Some(session.handshake().clone());
                self.session = Some(session);
                self.status = KernelStatus::Idle;
                Ok(())
            }
            Err(error) => {
                self.session = None;
                self.status = KernelStatus::Failed;
                Err(error)
            }
        }
    }

    fn execute(&mut self, req: &ExecuteRequest) -> Result<ExecuteResult, KernelError> {
        if self.status != KernelStatus::Idle {
            return Err(KernelError::InvalidStatus(self.status));
        }
        let session = self
            .session
            .as_mut()
            .ok_or(KernelError::InvalidStatus(self.status))?;

        self.status = KernelStatus::Busy;
        let result = session.execute(req);
        match &result {
            Ok(_) | Err(KernelError::Execution { .. }) | Err(KernelError::Interrupted) => {
                if session.check_alive().is_ok() {
                    self.status = KernelStatus::Idle;
                } else {
                    self.status = KernelStatus::Failed;
                }
            }
            Err(KernelError::Transport(_)) => {
                self.status = KernelStatus::Failed;
            }
            Err(_) => {
                if session.check_alive().is_ok() {
                    self.status = KernelStatus::Idle;
                } else {
                    self.status = KernelStatus::Failed;
                }
            }
        }
        result
    }

    fn interrupt(&mut self) -> Result<(), KernelError> {
        let Some(control) = self.control() else {
            return Err(KernelError::InvalidStatus(self.status));
        };
        control.interrupt()
    }

    fn control(&self) -> Option<Arc<dyn KernelControl>> {
        self.session.as_ref().map(JuliaSession::control_handle)
    }

    fn variables(&mut self) -> Result<Vec<Variable>, KernelError> {
        Ok(Vec::new())
    }

    fn shutdown(&mut self) -> Result<(), KernelError> {
        if let Some(mut session) = self.session.take() {
            let res = session.shutdown();
            self.status = KernelStatus::Stopped;
            return res;
        }
        self.status = KernelStatus::Stopped;
        Ok(())
    }

    fn restart(&mut self) -> Result<(), KernelError> {
        self.status = KernelStatus::Restarting;
        if let Some(mut session) = self.session.take() {
            let _ = session.shutdown();
        }
        self.start()
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
                    interrupt: true,
                    shutdown: true,
                    restart: true,
                },
            )
        })
    }

    fn export_value(&mut self, name: &str, profile: &ValueProfile) -> Result<Vec<u8>, KernelError> {
        if self.status != KernelStatus::Idle {
            return Err(KernelError::InvalidStatus(self.status));
        }
        if *profile != ValueProfile::scires_v1() {
            return Err(KernelError::UnsupportedValueProfile {
                id: profile.id.clone(),
                version: profile.version,
            });
        }
        let dp = self
            .session
            .as_ref()
            .and_then(JuliaSession::dataplane)
            .ok_or_else(|| KernelError::UnsupportedValueProfile {
                id: profile.id.clone(),
                version: profile.version,
            })?;
        dp.export_value(name)
    }

    fn import_value(
        &mut self,
        name: &str,
        profile: &ValueProfile,
        payload: &[u8],
    ) -> Result<(), KernelError> {
        if self.status != KernelStatus::Idle {
            return Err(KernelError::InvalidStatus(self.status));
        }
        if *profile != ValueProfile::scires_v1() {
            return Err(KernelError::UnsupportedValueProfile {
                id: profile.id.clone(),
                version: profile.version,
            });
        }
        let dp = self
            .session
            .as_ref()
            .and_then(JuliaSession::dataplane)
            .ok_or_else(|| KernelError::UnsupportedValueProfile {
                id: profile.id.clone(),
                version: profile.version,
            })?;
        dp.import_value(name, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scires_kernel::{KernelFactory, Output, RequestId};

    fn live_config() -> Option<JuliaKernelConfig> {
        let project = std::env::var_os("SCIRES_JULIA_PROJECT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .and_then(|p| p.parent())
                    .expect("workspace root")
                    .join("tests")
                    .join("julia")
            });
        JuliaKernelConfig::discover(project)
    }

    #[test]
    fn julia_factory_targets_julia() {
        let factory = JuliaKernelFactory::new(JuliaKernelConfig::new("julia", "tests/julia"));
        assert_eq!(factory.language(), Language::Julia);
    }

    #[test]
    fn julia_kernel_reports_versioned_handshake_with_interrupt_and_no_profiles_yet() {
        let kernel = JuliaKernel::new(JuliaKernelConfig::new("julia", "tests/julia"));
        let handshake = kernel.handshake();
        assert_eq!(handshake.api, "scires.kernel");
        assert_eq!(handshake.version, 1);
        assert_eq!(handshake.info.language, Language::Julia);
        assert!(handshake.capabilities.execute);
        assert!(handshake.capabilities.interrupt);
        assert!(handshake.capabilities.shutdown);
        assert!(handshake.capabilities.restart);
        assert!(handshake.value_profiles.is_empty());
        assert!(kernel.control().is_none());
    }

    #[test]
    fn start_with_missing_executable_transitions_to_failed() {
        let mut kernel = JuliaKernel::new(JuliaKernelConfig::new(
            "/nonexistent/scires-julia-binary",
            "tests/julia",
        ));
        let err = kernel.start().unwrap_err();
        assert!(matches!(err, KernelError::Startup(_)));
        assert_eq!(kernel.status(), KernelStatus::Failed);
    }

    #[test]
    fn execute_before_start_returns_invalid_status() {
        let mut kernel = JuliaKernel::new(JuliaKernelConfig::new("julia", "tests/julia"));
        assert_eq!(
            kernel.execute(&ExecuteRequest::new("1 + 1")),
            Err(KernelError::InvalidStatus(KernelStatus::NotStarted))
        );
    }

    #[test]
    #[ignore = "requires an instantiated Julia/IJulia project (set SCIRES_JULIA_PROJECT and optionally SCIRES_JULIA_EXE)"]
    fn live_ijulia_round_trip_interrupt_restart_and_shutdown() {
        let config = live_config().expect("Julia executable should be discoverable");
        let mut kernel = JuliaKernel::new(config);
        kernel.start().expect("IJulia kernel should start");
        assert_eq!(kernel.status(), KernelStatus::Idle);

        let handshake = kernel.handshake();
        assert_eq!(handshake.version, 1);
        assert_ne!(handshake.info.language_version, "unknown");
        assert!(handshake.capabilities.interrupt);

        let req = ExecuteRequest::new("scires_probe = 42; println(\"direct-zmq-ok\"); scires_probe")
            .with_request_id(RequestId::new("live-req-1"));
        let result = kernel.execute(&req).expect("Julia execution should succeed");
        assert_eq!(result.request_id, RequestId::new("live-req-1"));
        assert!(result
            .outputs
            .iter()
            .any(|o| matches!(o, Output::Stdout(s) if s.contains("direct-zmq-ok"))));
        assert!(result
            .outputs
            .iter()
            .any(|o| matches!(o, Output::Display { mime, data } if mime == "text/plain" && data.trim() == "42")));

        let err = kernel
            .execute(&ExecuteRequest::new("error(\"direct-zmq-error\")"))
            .expect_err("Julia error should surface");
        assert!(
            matches!(
                &err,
                KernelError::Execution { ename, evalue, traceback }
                    if !ename.is_empty()
                        && evalue.contains("direct-zmq-error")
                        && !traceback.is_empty()
            ),
            "unexpected error: {err:?}"
        );

        // Concurrent interrupt via KernelControl while execute("sleep(30)") is in flight.
        let control = kernel.control().expect("live kernel exposes control handle");
        let interrupt_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            control.interrupt()
        });
        let interrupted = kernel.execute(&ExecuteRequest::new("sleep(30)"));
        interrupt_thread
            .join()
            .expect("interrupt thread should join")
            .expect("interrupt_request should succeed");
        assert_eq!(interrupted, Err(KernelError::Interrupted));
        assert_eq!(kernel.status(), KernelStatus::Idle);

        // Restart clears namespace.
        kernel.restart().expect("restart should succeed");
        let after_restart = kernel
            .execute(&ExecuteRequest::new("isdefined(Main, :scires_probe)"))
            .expect("execution after restart should succeed");
        assert!(after_restart
            .outputs
            .iter()
            .any(|o| matches!(o, Output::Display { mime, data } if mime == "text/plain" && data.trim() == "false")));

        kernel.shutdown().expect("shutdown should succeed");
        assert_eq!(kernel.status(), KernelStatus::Stopped);
    }
}
