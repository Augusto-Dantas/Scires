//! In-process test double kernel that echoes code back. Useful for tests and demos.

use crate::{
    ExecuteRequest, ExecuteResult, Kernel, KernelCapabilities, KernelControl, KernelError,
    KernelFactory, KernelHandshake, KernelInfo, KernelStatus, Output, ValueProfile,
};
use scires_core::{Language, Value, Variable};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct EchoKernelControl {
    interrupted: Arc<AtomicBool>,
}

impl KernelControl for EchoKernelControl {
    fn interrupt(&self) -> Result<(), KernelError> {
        self.interrupted.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Kernel that echoes code to stdout, treats `name = int` lines as assignments,
/// supports out-of-band interrupt via [`KernelControl`], and stores opaque
/// v1 value-profile payloads in memory.
pub struct EchoKernel {
    language: Language,
    status: KernelStatus,
    count: u64,
    vars: Vec<Variable>,
    values: HashMap<String, Vec<u8>>,
    interrupted: Arc<AtomicBool>,
}

impl EchoKernel {
    /// Create a not-started echo kernel.
    pub fn new(language: Language) -> Self {
        Self {
            language,
            status: KernelStatus::NotStarted,
            count: 0,
            vars: Vec::new(),
            values: HashMap::new(),
            interrupted: Arc::new(AtomicBool::new(false)),
        }
    }

    fn ensure_idle(&self) -> Result<(), KernelError> {
        match self.status {
            KernelStatus::Idle => Ok(()),
            s => Err(KernelError::InvalidStatus(s)),
        }
    }

    fn ensure_supported_profile(profile: &ValueProfile) -> Result<(), KernelError> {
        if *profile == ValueProfile::scires_v1() {
            Ok(())
        } else {
            Err(KernelError::UnsupportedValueProfile {
                id: profile.id.clone(),
                version: profile.version,
            })
        }
    }
}

impl Kernel for EchoKernel {
    fn info(&self) -> KernelInfo {
        KernelInfo {
            language: self.language.clone(),
            implementation: "echo".into(),
            language_version: "0".into(),
        }
    }

    fn status(&self) -> KernelStatus {
        self.status
    }

    fn start(&mut self) -> Result<(), KernelError> {
        self.interrupted.store(false, Ordering::SeqCst);
        self.status = KernelStatus::Idle;
        Ok(())
    }

    fn execute(&mut self, req: &ExecuteRequest) -> Result<ExecuteResult, KernelError> {
        self.ensure_idle()?;
        if self.interrupted.swap(false, Ordering::SeqCst) {
            return Err(KernelError::Interrupted);
        }
        if req.code.trim_start().starts_with("error(") {
            return Err(KernelError::Execution {
                ename: "ErrorException".into(),
                evalue: req.code.clone(),
                traceback: vec![format!("ErrorException: {}", req.code)],
            });
        }
        if let Some((name, val)) = req.code.split_once('=') {
            if let Ok(i) = val.trim().parse::<i64>() {
                let name = name.trim().to_owned();
                self.vars.retain(|v| v.name != name);
                self.vars
                    .push(Variable::new(name, self.language.clone(), Value::Int(i)));
            }
        }
        self.count += 1;
        Ok(ExecuteResult {
            request_id: req.request_id.clone(),
            execution_count: self.count,
            outputs: if req.silent {
                vec![]
            } else {
                vec![Output::Stdout(req.code.clone())]
            },
        })
    }

    fn interrupt(&mut self) -> Result<(), KernelError> {
        self.interrupted.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn control(&self) -> Option<Arc<dyn KernelControl>> {
        Some(Arc::new(EchoKernelControl {
            interrupted: Arc::clone(&self.interrupted),
        }))
    }

    fn variables(&mut self) -> Result<Vec<Variable>, KernelError> {
        Ok(self.vars.clone())
    }

    fn shutdown(&mut self) -> Result<(), KernelError> {
        self.status = KernelStatus::Stopped;
        Ok(())
    }

    fn restart(&mut self) -> Result<(), KernelError> {
        self.status = KernelStatus::Idle;
        self.count = 0;
        self.vars.clear();
        self.values.clear();
        self.interrupted.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn handshake(&self) -> KernelHandshake {
        KernelHandshake::new(self.info(), KernelCapabilities::default())
            .with_value_profiles(vec![ValueProfile::scires_v1()])
    }

    fn export_value(&mut self, name: &str, profile: &ValueProfile) -> Result<Vec<u8>, KernelError> {
        self.ensure_idle()?;
        Self::ensure_supported_profile(profile)?;
        self.values
            .get(name)
            .cloned()
            .ok_or_else(|| KernelError::Transfer(format!("variable `{name}` is not defined")))
    }

    fn import_value(
        &mut self,
        name: &str,
        profile: &ValueProfile,
        payload: &[u8],
    ) -> Result<(), KernelError> {
        self.ensure_idle()?;
        Self::ensure_supported_profile(profile)?;
        self.values.insert(name.to_owned(), payload.to_vec());
        Ok(())
    }
}

/// Factory for [`EchoKernel`].
pub struct EchoKernelFactory(Language);

impl EchoKernelFactory {
    /// Factory for the given language.
    pub fn new(language: Language) -> Self {
        Self(language)
    }
}

impl KernelFactory for EchoKernelFactory {
    fn language(&self) -> Language {
        self.0.clone()
    }

    fn create(&self) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(EchoKernel::new(self.0.clone())))
    }
}
