use crate::{BreakpointStore, DebugAdapter, DebugError, StackFrame};
use scires_core::Variable;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Why execution stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    /// Hit a breakpoint.
    Breakpoint,
    /// Finished a step.
    Step,
    /// User requested pause.
    Pause,
    /// Uncaught exception (post-mortem debugging).
    Exception,
}

/// Debug session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DebugState {
    /// Not launched.
    Idle,
    /// Debuggee running.
    Running,
    /// Debuggee stopped.
    Paused(StopReason),
    /// Session finished.
    Terminated,
}

/// A debug session wrapping an adapter and enforcing valid transitions.
pub struct DebugSession {
    adapter: Box<dyn DebugAdapter>,
    state: DebugState,
}

macro_rules! require_paused {
    ($self:ident, $cmd:literal) => {
        if !matches!($self.state, DebugState::Paused(_)) {
            return Err(DebugError::InvalidTransition {
                command: $cmd,
                state: $self.state,
            });
        }
    };
}

impl DebugSession {
    /// New idle session.
    pub fn new(adapter: Box<dyn DebugAdapter>) -> Self {
        Self {
            adapter,
            state: DebugState::Idle,
        }
    }

    /// Current state.
    pub fn state(&self) -> DebugState {
        self.state
    }

    /// Launch a program, syncing all enabled breakpoints first.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::InvalidTransition`] if the session is not [`DebugState::Idle`],
    /// or [`DebugError::Adapter`] if the underlying [`DebugAdapter`] fails.
    pub fn launch(&mut self, program: &Path, bps: &BreakpointStore) -> Result<(), DebugError> {
        if self.state != DebugState::Idle {
            return Err(DebugError::InvalidTransition {
                command: "launch",
                state: self.state,
            });
        }
        for src in bps.sources() {
            self.adapter.set_breakpoints(&src, &bps.active_for(&src))?;
        }
        self.adapter.launch(program)?;
        self.state = DebugState::Running;
        Ok(())
    }

    /// Called by the adapter event loop when the debuggee stops.
    pub fn on_stopped(&mut self, reason: StopReason) {
        if self.state == DebugState::Running {
            self.state = DebugState::Paused(reason);
        }
    }

    /// Called when the debuggee exits.
    pub fn on_exited(&mut self) {
        self.state = DebugState::Terminated;
    }

    /// Continue execution.
    pub fn resume(&mut self) -> Result<(), DebugError> {
        require_paused!(self, "continue");
        self.adapter.resume()?;
        self.state = DebugState::Running;
        Ok(())
    }

    /// Pause a running debuggee.
    pub fn pause(&mut self) -> Result<(), DebugError> {
        if self.state != DebugState::Running {
            return Err(DebugError::InvalidTransition {
                command: "pause",
                state: self.state,
            });
        }
        self.adapter.pause()
    }

    /// Step over.
    pub fn step_over(&mut self) -> Result<(), DebugError> {
        require_paused!(self, "step over");
        self.adapter.step_over()?;
        self.state = DebugState::Running;
        Ok(())
    }

    /// Step in.
    pub fn step_in(&mut self) -> Result<(), DebugError> {
        require_paused!(self, "step in");
        self.adapter.step_in()?;
        self.state = DebugState::Running;
        Ok(())
    }

    /// Step out.
    pub fn step_out(&mut self) -> Result<(), DebugError> {
        require_paused!(self, "step out");
        self.adapter.step_out()?;
        self.state = DebugState::Running;
        Ok(())
    }

    /// Call stack (only while paused).
    pub fn stack_trace(&mut self) -> Result<Vec<StackFrame>, DebugError> {
        require_paused!(self, "stack trace");
        self.adapter.stack_trace()
    }

    /// Variables of a frame (only while paused).
    pub fn variables(&mut self, frame_id: u64) -> Result<Vec<Variable>, DebugError> {
        require_paused!(self, "variables");
        self.adapter.variables(frame_id)
    }

    /// Evaluate in a frame (only while paused).
    pub fn evaluate(&mut self, expr: &str, frame_id: u64) -> Result<String, DebugError> {
        require_paused!(self, "evaluate");
        self.adapter.evaluate(expr, frame_id)
    }

    /// Terminate the session (idempotent).
    pub fn terminate(&mut self) -> Result<(), DebugError> {
        if matches!(self.state, DebugState::Running | DebugState::Paused(_)) {
            self.adapter.terminate()?;
        }
        self.state = DebugState::Terminated;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Breakpoint;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    struct FakeAdapter {
        log: Arc<Mutex<Vec<String>>>,
    }

    impl FakeAdapter {
        fn push(&self, s: impl Into<String>) -> Result<(), DebugError> {
            self.log.lock().unwrap().push(s.into());
            Ok(())
        }
    }

    impl DebugAdapter for FakeAdapter {
        fn launch(&mut self, _: &Path) -> Result<(), DebugError> {
            self.push("launch")
        }
        fn set_breakpoints(&mut self, _: &Path, bps: &[Breakpoint]) -> Result<(), DebugError> {
            self.push(format!("bps:{}", bps.len()))
        }
        fn resume(&mut self) -> Result<(), DebugError> {
            self.push("resume")
        }
        fn pause(&mut self) -> Result<(), DebugError> {
            self.push("pause")
        }
        fn step_over(&mut self) -> Result<(), DebugError> {
            self.push("next")
        }
        fn step_in(&mut self) -> Result<(), DebugError> {
            self.push("in")
        }
        fn step_out(&mut self) -> Result<(), DebugError> {
            self.push("out")
        }
        fn stack_trace(&mut self) -> Result<Vec<StackFrame>, DebugError> {
            Ok(vec![])
        }
        fn variables(&mut self, _: u64) -> Result<Vec<Variable>, DebugError> {
            Ok(vec![])
        }
        fn evaluate(&mut self, e: &str, _: u64) -> Result<String, DebugError> {
            Ok(e.to_owned())
        }
        fn terminate(&mut self) -> Result<(), DebugError> {
            self.push("terminate")
        }
    }

    #[test]
    fn full_lifecycle() {
        let fake = FakeAdapter::default();
        let log = fake.log.clone();
        let mut bps = BreakpointStore::new();
        bps.add("main.jl", 10, None).unwrap();

        let mut s = DebugSession::new(Box::new(fake));
        s.launch(Path::new("main.jl"), &bps).unwrap();
        assert_eq!(s.state(), DebugState::Running);

        assert!(s.step_over().is_err(), "cannot step while running");
        s.on_stopped(StopReason::Breakpoint);
        assert_eq!(s.evaluate("x", 0).unwrap(), "x");
        s.step_over().unwrap();
        s.terminate().unwrap();
        assert_eq!(s.state(), DebugState::Terminated);
        assert_eq!(
            *log.lock().unwrap(),
            vec!["bps:1", "launch", "next", "terminate"]
        );
    }

    #[test]
    fn cannot_launch_twice() {
        let mut s = DebugSession::new(Box::new(FakeAdapter::default()));
        let bps = BreakpointStore::new();
        s.launch(Path::new("a.jl"), &bps).unwrap();
        assert!(s.launch(Path::new("a.jl"), &bps).is_err());
    }
}
