//! In-process test double kernel that echoes code back. Useful for tests and demos.

use crate::{
    ExecuteRequest, ExecuteResult, Kernel, KernelError, KernelFactory, KernelInfo, KernelStatus,
    Output,
};
use scires_core::{Language, Value, Variable};

/// Kernel that echoes code to stdout and treats `name = int` lines as assignments.
pub struct EchoKernel {
    language: Language,
    status: KernelStatus,
    count: u64,
    vars: Vec<Variable>,
}

impl EchoKernel {
    /// Create a not-started echo kernel.
    pub fn new(language: Language) -> Self {
        Self {
            language,
            status: KernelStatus::NotStarted,
            count: 0,
            vars: Vec::new(),
        }
    }

    fn ensure_idle(&self) -> Result<(), KernelError> {
        match self.status {
            KernelStatus::Idle => Ok(()),
            s => Err(KernelError::InvalidStatus(s)),
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
        self.status = KernelStatus::Idle;
        Ok(())
    }

    fn execute(&mut self, req: &ExecuteRequest) -> Result<ExecuteResult, KernelError> {
        self.ensure_idle()?;
        if req.code.trim_start().starts_with("error(") {
            return Err(KernelError::Execution {
                ename: "ErrorException".into(),
                evalue: req.code.clone(),
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
            execution_count: self.count,
            outputs: if req.silent {
                vec![]
            } else {
                vec![Output::Stdout(req.code.clone())]
            },
        })
    }

    fn interrupt(&mut self) -> Result<(), KernelError> {
        Ok(())
    }

    fn variables(&mut self) -> Result<Vec<Variable>, KernelError> {
        Ok(self.vars.clone())
    }

    fn shutdown(&mut self) -> Result<(), KernelError> {
        self.status = KernelStatus::Dead;
        Ok(())
    }

    fn restart(&mut self) -> Result<(), KernelError> {
        self.status = KernelStatus::Idle;
        self.count = 0;
        self.vars.clear();
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
